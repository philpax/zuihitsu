//! The console's WASM bridge.
//!
//! A [`Replica`] holds an event log and the graph it folds into, using `zuihitsu-core`'s real
//! materializer — the same projection the live agent runs (see `console/PLAN.md`). The frontend
//! constructs one from a run's `Event[]` (a stored run's events, or a live `/control` stream) and
//! queries it for the State and Time-travel views. The event-stream views (Events, Conversation)
//! and the surrounding chrome render off the JSON directly, so they need nothing here.
//!
//! The boundary discipline: events come in as raw JSON bytes parsed by `serde` *inside* the module
//! (one copy across the boundary), and results go out through `serde-wasm-bindgen`'s JSON-compatible
//! serializer, so numbers land as JS numbers rather than `BigInt` — matching the ts-rs bindings,
//! which type `Seq` and the timestamps as `number`.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use ulid::Ulid;
use wasm_bindgen::prelude::*;
use zuihitsu_core::{
    brief::{BriefRequest, compose_traced},
    event::{Event, EventPayload, MergeProposalSource},
    graph::{EntryView, Graph, LinkView, MemoryView},
    ids::{ConversationId, EntryId, MemoryId, MemoryName, Namespace, Seq, SessionId, TurnId},
    settings::BriefSettings,
    time::{MILLIS_PER_DAY, Timestamp},
    turn_ref,
};

/// How many instances of a single recurring rule the agenda expands within its horizon — a bound so
/// a daily rule cannot flood the view (a weekly or monthly rule stays well under it over the
/// horizon).
const MAX_RECURRING_INSTANCES: usize = 20;

/// Everything the State view shows when a memory is opened: the memory itself, its live content
/// entries, its full history (including superseded entries), its links, and its `same_as` class.
/// Composed from several core reads so the frontend opens a memory in one call.
#[derive(Serialize)]
struct MemoryDetail {
    memory: MemoryView,
    entries: Vec<EntryView>,
    history: Vec<EntryView>,
    links: Vec<LinkView>,
    class: Vec<MemoryView>,
    /// The entry ids currently under an unresolved belief arbitration, so the view can mark a contested
    /// fact as disputed (the same signal the agent sees on a read).
    disputed: Vec<EntryId>,
}

/// One cross-platform merge proposal as the console surfaces it (spec §Cross-platform identity →
/// adjudicated merge): the two stubs by handle *and* id (so the view can name them and deep-link into
/// State), who raised it, the proposer's stated grounds if any, and where the proposal now stands. Unlike
/// the operator backstop — which drops a settled proposal — the console keeps every proposal so it can
/// show the whole adjudication record: what identity calls were made and which still await one.
#[derive(Serialize)]
struct MergeProposalView {
    from: MemoryName,
    to: MemoryName,
    from_id: MemoryId,
    to_id: MemoryId,
    source: MergeProposalSource,
    /// The proposer's stated grounds for the match — the coincidence the agent reasoned from. `None` for
    /// an orchestration handle match or a `same_as`-via-link, which carry no rationale.
    rationale: Option<String>,
    status: MergeStatus,
}

/// Where a merge proposal stands at the current fold horizon: still awaiting a decision, merged (the two
/// stubs now share a `same_as` class, whether an adjudication or an operator authored it), or rejected (an
/// adjudication or an operator refused it, and the stubs stay distinct).
#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum MergeStatus {
    Pending,
    Merged,
    Rejected,
}

/// One item on the agent's agenda: when it occurs, the memory it lives in, the text, and whether it
/// is a recurring instance. One-offs come from `occurrences_in_window`; recurring instances from
/// `recurring_instances_in_window`, which expands each rule through the agent's own `next_occurrence`
/// so the projection cannot drift from the agent's scheduling.
#[derive(Serialize)]
struct AgendaItem {
    when: Timestamp,
    /// The occurrence is a whole day or fuzzier span, not a precise instant, so the calendar renders
    /// it without a clock time (a `Day` sorts at noon — not a stated time). See `TemporalRef::is_all_day`.
    all_day: bool,
    memory: String,
    text: String,
    recurring: bool,
}

/// A durable conversation (room) with its sessions, the backbone of the Conversation view. The
/// turns themselves render off the event stream; this supplies the structure and the names the raw
/// log only carries as ids — the room's [`Namespace::Context`] name and each session's participant
/// handles.
#[derive(Serialize)]
struct ConversationDetail {
    id: ConversationId,
    platform: String,
    scope_path: String,
    context_name: Option<String>,
    sessions: Vec<SessionSummary>,
}

/// One activity window within a conversation: when it opened, the brief frozen at its start, and the
/// participants present, resolved to their memory handles.
#[derive(Serialize)]
struct SessionSummary {
    id: SessionId,
    started_at: Timestamp,
    brief: String,
    participants: Vec<String>,
}

/// One span of scanned turn-reference text going out to JS: literal prose, or a reference resolved
/// to its turn's ULID string. Mirrors `turn_ref::Segment` in a serde-friendly shape (the core enum
/// borrows and carries a typed id; the crossing wants owned strings).
#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum RefSegment<'a> {
    Prose { text: &'a str },
    Ref { id: String },
}

/// Split `text` into prose spans and turn references — the console's pretty projection runs each
/// turn's text through this so a `[turn:<ulid>]` token or a pasted deep-link URL renders as a chip.
/// The parser is `zuihitsu_core::turn_ref::scan`, the same definition the agent's resolver reads, so
/// what the console highlights and what the agent resolves cannot drift.
#[wasm_bindgen(js_name = turnRefScan)]
pub fn turn_ref_scan(text: &str) -> Result<JsValue, JsError> {
    let segments: Vec<RefSegment> = turn_ref::scan(text)
        .into_iter()
        .map(|segment| match segment {
            turn_ref::Segment::Prose(prose) => RefSegment::Prose { text: prose },
            turn_ref::Segment::Ref(turn) => RefSegment::Ref {
                id: turn.0.to_string(),
            },
        })
        .collect();
    to_js(&segments)
}

/// Rebuild `text` with every turn reference rendered as the canonical `[turn:<ulid>]` token — the
/// composer's send-time normalization, so a pasted console URL leaves the console as ref syntax and
/// every downstream consumer sees one form.
#[wasm_bindgen(js_name = turnRefNormalize)]
pub fn turn_ref_normalize(text: &str) -> String {
    turn_ref::normalize(text)
}

/// Every turn id referenced in `text`, in order of appearance — the extract-all-ids path.
#[wasm_bindgen(js_name = turnRefExtract)]
pub fn turn_ref_extract(text: &str) -> Result<JsValue, JsError> {
    let ids: Vec<String> = turn_ref::extract_ids(text)
        .into_iter()
        .map(|turn| turn.0.to_string())
        .collect();
    to_js(&ids)
}

/// The canonical `[turn:<ulid>]` token for a turn id, or an error if `id` is not a ULID — so the
/// console mints citations through the same constructor the agent's `ref` field uses.
#[wasm_bindgen(js_name = turnRefConstruct)]
pub fn turn_ref_construct(id: &str) -> Result<String, JsError> {
    let ulid = Ulid::from_string(id)
        .map_err(|error| JsError::new(&format!("console: parsing the turn id {id:?}: {error}")))?;
    Ok(turn_ref::construct(TurnId(ulid)))
}

/// A materializing read replica: an event log plus the graph state it folds into. The log is
/// retained so the graph can be re-folded to any earlier `Seq` for time-travel.
#[wasm_bindgen]
pub struct Replica {
    events: Vec<Event>,
    graph: Graph,
    /// The highest `Seq` currently folded into `graph` (the fold horizon).
    head: Seq,
}

#[wasm_bindgen]
impl Replica {
    /// Build a replica from a JSON-encoded `Event[]` — a run's embedded log, or a live catch-up
    /// batch. The events are folded through the real materializer up to their head.
    #[wasm_bindgen(constructor)]
    pub fn new(events_json: &[u8]) -> Result<Replica, JsError> {
        let mut events: Vec<Event> = serde_json::from_slice(events_json)
            .map_err(|error| JsError::new(&format!("console: parsing the event log: {error}")))?;
        events.sort_by_key(|event| event.seq);

        let mut replica = Replica {
            events,
            graph: Graph::open_in_memory().map_err(graph_error)?,
            head: Seq::ZERO,
        };
        let head = replica
            .events
            .last()
            .map(|event| event.seq)
            .unwrap_or(Seq::ZERO);
        replica.fold_through(head)?;
        Ok(replica)
    }

    /// Append a JSON-encoded `Event[]` tail to the log without re-folding — the live console's
    /// catch-up poll (spec §Observability → live phase). New events are merged in `seq` order; any
    /// at or below the current log head are dropped as a poll-overlap re-delivery. The fold horizon
    /// is left untouched, so the caller chooses whether to advance it (follow the head) or hold it
    /// (time-travel pinned) with a subsequent `foldTo`.
    pub fn append(&mut self, events_json: &[u8]) -> Result<(), JsError> {
        let mut incoming: Vec<Event> = serde_json::from_slice(events_json)
            .map_err(|error| JsError::new(&format!("console: parsing the event tail: {error}")))?;
        let log_head = self
            .events
            .last()
            .map(|event| event.seq)
            .unwrap_or(Seq::ZERO);
        incoming.retain(|event| event.seq > log_head);
        incoming.sort_by_key(|event| event.seq);
        self.events.extend(incoming);
        Ok(())
    }

    /// The number of events in the log (independent of the fold horizon).
    #[wasm_bindgen(getter, js_name = eventCount)]
    pub fn event_count(&self) -> usize {
        self.events.len()
    }

    /// The highest `Seq` in the log — the upper bound of the time-travel range.
    #[wasm_bindgen(getter, js_name = headSeq)]
    pub fn head_seq(&self) -> f64 {
        self.events
            .last()
            .map(|event| event.seq.0 as f64)
            .unwrap_or(0.0)
    }

    /// The `Seq` currently folded into the graph (what the queries below reflect).
    #[wasm_bindgen(getter, js_name = foldedSeq)]
    pub fn folded_seq(&self) -> f64 {
        self.head.0 as f64
    }

    /// Re-fold the graph to reflect only events with `seq <= up_to` — the time-travel scrub. Folding
    /// from zero each time is fine at a run's scale; caching checkpoints is a later optimization.
    #[wasm_bindgen(js_name = foldTo)]
    pub fn fold_to(&mut self, up_to: f64) -> Result<(), JsError> {
        self.fold_through(Seq(up_to.max(0.0) as u64))
    }

    /// Every memory at the current fold horizon, as `MemoryView[]`, ordered by name. Pass a `prefix`
    /// (e.g. `"person/"`) to scope by namespace, or an empty string for all.
    pub fn memories(&self, prefix: &str) -> Result<JsValue, JsError> {
        let mut memories = self
            .graph
            .memories_in_namespace(prefix)
            .map_err(graph_error)?;
        memories.sort_by(|a, b| a.name.as_str().cmp(b.name.as_str()));
        to_js(&memories)
    }

    /// The full State-view detail for one memory by name, or `null` if there is no such memory at
    /// the current fold horizon. Bundles its live entries, its history, its links, and its `same_as`
    /// class so the frontend opens a memory in a single call.
    pub fn memory(&self, name: &str) -> Result<JsValue, JsError> {
        let Some(memory) = self
            .graph
            .memory_by_name(MemoryName::new(name))
            .map_err(graph_error)?
        else {
            return Ok(JsValue::NULL);
        };
        let entries = self.graph.entries_local(memory.id).map_err(graph_error)?;
        let history = self
            .graph
            .entries_local_history(memory.id)
            .map_err(graph_error)?;
        let links = self.graph.links(memory.id).map_err(graph_error)?;
        let mut class = Vec::new();
        for id in self.graph.class_members(memory.id).map_err(graph_error)? {
            if let Some(view) = self.graph.memory_by_id(id).map_err(graph_error)? {
                class.push(view);
            }
        }
        let disputed = self
            .graph
            .disputed_entries(memory.id)
            .map_err(graph_error)?
            .into_iter()
            .collect();
        to_js(&MemoryDetail {
            memory,
            entries,
            history,
            links,
            class,
            disputed,
        })
    }

    /// The tag vocabulary at the current fold horizon, as `TagVocabularyEntry[]` (name, purpose, and
    /// live-use count).
    pub fn tags(&self) -> Result<JsValue, JsError> {
        to_js(&self.graph.all_tags().map_err(graph_error)?)
    }

    /// The registered link relations at the current fold horizon, as `RelationView[]`.
    pub fn relations(&self) -> Result<JsValue, JsError> {
        to_js(&self.graph.all_relations().map_err(graph_error)?)
    }

    /// Every cross-platform merge proposal in the folded log, in first-proposal order, each tagged with
    /// where it now stands — pending, merged, or rejected (spec §Cross-platform identity → adjudicated
    /// merge). `MergeProposed`/`MergeAdjudicated` are log-only, so this reads them from the events (up to
    /// the fold horizon) and resolves the resolution: an accepted or operator merge shows as a shared
    /// `same_as` class in the graph, a refusal as the latest `MergeAdjudicated` verdict, everything else
    /// as still pending. A pair is keyed by its canonical (order-independent) form since `same_as` is
    /// symmetric, but the first proposal's direction and stated grounds are kept for a stable display.
    #[wasm_bindgen(js_name = mergeProposals)]
    pub fn merge_proposals(&self) -> Result<JsValue, JsError> {
        let mut order: Vec<(MemoryId, MemoryId)> = Vec::new();
        let mut source: BTreeMap<(MemoryId, MemoryId), MergeProposalSource> = BTreeMap::new();
        let mut rationale: BTreeMap<(MemoryId, MemoryId), Option<String>> = BTreeMap::new();
        // The pairs whose latest adjudication verdict refused the merge; an accept clears it.
        let mut refused: BTreeSet<(MemoryId, MemoryId)> = BTreeSet::new();
        for event in self.events.iter().filter(|event| event.seq <= self.head) {
            match &event.payload {
                EventPayload::MergeProposed {
                    from,
                    to,
                    source: raised_by,
                    rationale: grounds,
                } => {
                    let pair = canonical_pair(*from, *to);
                    match rationale.get_mut(&pair) {
                        // A later proposal fills a rationale an earlier bare one lacked; a bare
                        // re-proposal never erases stated grounds already recorded.
                        Some(existing) if existing.is_none() && grounds.is_some() => {
                            *existing = grounds.clone();
                        }
                        Some(_) => {}
                        None => {
                            order.push((*from, *to));
                            source.insert(pair, *raised_by);
                            rationale.insert(pair, grounds.clone());
                        }
                    }
                }
                EventPayload::MergeAdjudicated {
                    from, to, accepted, ..
                } => {
                    let pair = canonical_pair(*from, *to);
                    if *accepted {
                        refused.remove(&pair);
                    } else {
                        refused.insert(pair);
                    }
                }
                _ => {}
            }
        }

        let name = |id: MemoryId| -> Result<MemoryName, JsError> {
            Ok(self
                .graph
                .memory_by_id(id)
                .map_err(graph_error)?
                .map(|memory| memory.name)
                .unwrap_or_else(|| MemoryName::new("<unknown>")))
        };
        let mut out = Vec::new();
        for (from, to) in order {
            let pair = canonical_pair(from, to);
            let from_class = self.graph.class_id(from).map_err(graph_error)?;
            let merged = from_class.is_some()
                && from_class == self.graph.class_id(to).map_err(graph_error)?;
            let status = if merged {
                MergeStatus::Merged
            } else if refused.contains(&pair) {
                MergeStatus::Rejected
            } else {
                MergeStatus::Pending
            };
            out.push(MergeProposalView {
                from: name(from)?,
                to: name(to)?,
                from_id: from,
                to_id: to,
                source: source[&pair],
                rationale: rationale[&pair].clone(),
                status,
            });
        }
        to_js(&out)
    }

    /// Re-derive a session's contextual brief and the trace of how it was composed — every memory the
    /// composer considered and, per entry, the visibility verdict and whether it reached the brief.
    /// The inputs are the session's present set (memory ids), its room's [`Namespace::Context`]
    /// memory (if any), and its start time; the brief is composed against the graph at the current
    /// fold horizon.
    pub fn brief(
        &self,
        present_set: Vec<String>,
        context: Option<String>,
        now_ms: f64,
    ) -> Result<JsValue, JsError> {
        let present = present_set
            .iter()
            .map(|id| parse_memory_id(id))
            .collect::<Result<Vec<_>, _>>()?;
        let current_context = match context {
            Some(id) => Some(parse_memory_id(&id)?),
            None => None,
        };
        let request = BriefRequest {
            present_set: &present,
            current_context,
            working_set: &[],
            now: Timestamp::from_millis(now_ms as i64),
        };
        let trace = compose_traced(&self.graph, &BriefSettings::default(), &request)
            .map_err(|error| JsError::new(&format!("console: {error}")))?;
        to_js(&trace)
    }

    /// The agent's upcoming agenda from `now_ms`: **all** future one-off dated occurrences (a thing
    /// set three months out stays visible), plus recurring entries *expanded* into every instance
    /// within `horizon_days` (each rule capped at [`MAX_RECURRING_INSTANCES`] so a daily one cannot
    /// flood it) — recurring needs a horizon since it is unbounded, one-offs do not. Merged and
    /// ordered soonest first. Each recurring instance comes from the agent's own `next_occurrence`,
    /// so the console never reimplements RRULE expansion and cannot drift from the agent's calendar.
    pub fn agenda(&self, now_ms: f64, horizon_days: f64) -> Result<JsValue, JsError> {
        let from = Timestamp::from_millis(now_ms as i64);
        let horizon =
            Timestamp::from_millis(now_ms as i64 + (horizon_days as i64) * MILLIS_PER_DAY);
        let mut items = Vec::new();
        // One-offs are finite, so they have no upper bound — a far-future event stays on the agenda.
        for (memory, entry) in self
            .graph
            .occurrences_in_window(from, Timestamp::from_millis(i64::MAX))
            .map_err(graph_error)?
        {
            items.push(AgendaItem {
                when: entry.occurred_sort.unwrap_or(from),
                all_day: entry.occurred_at.as_ref().is_none_or(|at| at.is_all_day()),
                memory: memory.name.as_str().to_owned(),
                text: entry.text,
                recurring: false,
            });
        }
        for (instant, memory, text) in self
            .graph
            .recurring_instances_in_window(from, horizon, MAX_RECURRING_INSTANCES)
            .map_err(graph_error)?
        {
            items.push(AgendaItem {
                // The supported rrule subset (FREQ + INTERVAL) carries no time of day, so a recurring
                // instance is day-granular — its clock time would be the incidental anchor time.
                when: instant,
                all_day: true,
                memory: memory.name.as_str().to_owned(),
                text,
                recurring: true,
            });
        }
        items.sort_by_key(|item| item.when.as_millis());
        to_js(&items)
    }

    /// Every durable conversation up to the current fold horizon, each with its sessions — the
    /// structure behind the Conversation view, with the [`Namespace::Context`] room name and the
    /// per-session participant handles resolved from ids the raw log only carries opaquely.
    pub fn conversations(&self) -> Result<JsValue, JsError> {
        let mut conversations = Vec::new();
        for event in self.events.iter().filter(|event| event.seq <= self.head) {
            let EventPayload::ConversationStarted {
                id,
                locator,
                context_memory,
            } = &event.payload
            else {
                continue;
            };
            let context_name = self
                .graph
                .memory_by_id(*context_memory)
                .map_err(graph_error)?
                .map(|view| view.name.as_str().to_owned());
            let mut sessions = Vec::new();
            for session in self.graph.sessions_in(*id).map_err(graph_error)? {
                let mut participants = Vec::new();
                for participant in &session.participants {
                    if let Some(view) =
                        self.graph.memory_by_id(*participant).map_err(graph_error)?
                    {
                        participants.push(view.name.as_str().to_owned());
                    }
                }
                sessions.push(SessionSummary {
                    id: session.id,
                    started_at: session.started_at,
                    brief: session.brief,
                    participants,
                });
            }
            conversations.push(ConversationDetail {
                id: *id,
                platform: locator.platform.to_string(),
                scope_path: locator.scope_path.to_string(),
                context_name,
                sessions,
            });
        }
        to_js(&conversations)
    }

    /// The memory name a freshly minted [`Namespace::Person`] participant would receive, given
    /// their platform handle and the platform they arrived on. Delegates to the graph's own
    /// name-resolution logic
    /// (the same path the server's mint uses), so the optimistic preview shows the exact name the
    /// real turn will resolve to — including the `@platform` disambiguation on collision.
    pub fn participant_name(
        &self,
        platform: &str,
        platform_user_id: &str,
    ) -> Result<JsValue, JsError> {
        let name = self
            .graph
            .participant_name(platform, platform_user_id)
            .map_err(graph_error)?;
        to_js(&name)
    }

    /// All live [`Namespace::Person`] memories at the current fold horizon — the namespace prefix
    /// is owned by Rust ([`Namespace::Person`]), so the frontend never hardcodes `person/` to
    /// scope the query.
    pub fn person_memories(&self) -> Result<JsValue, JsError> {
        let mut memories = self
            .graph
            .memories_in_namespace(Namespace::Person.prefix())
            .map_err(graph_error)?;
        memories.sort_by(|a, b| a.name.as_str().cmp(b.name.as_str()));
        to_js(&memories)
    }

    /// The platform user ids seen on a given platform — the bare handles a user can type in the "you
    /// are" field, sourced from `participant_identities` so the `@platform` disambiguation suffix
    /// never surfaces as a separate entry.
    pub fn participant_ids(&self, platform: &str) -> Result<JsValue, JsError> {
        let ids = self
            .graph
            .participant_ids_for(platform)
            .map_err(graph_error)?;
        to_js(&ids)
    }

    /// Decompose a memory name into its namespace and subject (e.g. `person/dave` → `Person` +
    /// `"dave"`), or `null` if the name is in no known namespace (e.g. `self`). The parse uses
    /// [`Namespace`] internally, so the frontend never hardcodes the prefix strings.
    pub fn parse_name(&self, name: &str) -> Result<JsValue, JsError> {
        match MemoryName::new(name).namespaced() {
            Ok(namespaced) => to_js(&namespaced),
            Err(_) => Ok(JsValue::NULL),
        }
    }
}

impl Replica {
    /// Rebuild `graph` from a fresh in-memory projection, applying every event with `seq <= up_to`.
    fn fold_through(&mut self, up_to: Seq) -> Result<(), JsError> {
        let mut graph = Graph::open_in_memory().map_err(graph_error)?;
        for event in self.events.iter().filter(|event| event.seq <= up_to) {
            graph.apply(event).map_err(graph_error)?;
        }
        self.graph = graph;
        self.head = up_to;
        Ok(())
    }
}

/// Serialize a value to a JS value with the JSON-compatible number policy (plain numbers, not
/// `BigInt`), so the result matches the ts-rs bindings the frontend is typed against.
fn to_js<T: Serialize>(value: &T) -> Result<JsValue, JsError> {
    value
        .serialize(&serde_wasm_bindgen::Serializer::json_compatible())
        .map_err(|error| JsError::new(&format!("console: serializing a result: {error}")))
}

/// Render a core graph error as a JS error, leading with the console context.
fn graph_error(error: zuihitsu_core::graph::GraphError) -> JsError {
    JsError::new(&format!("console: {error}"))
}

/// Order a merge pair so `(a, b)` and `(b, a)` coalesce — `same_as` is symmetric, so a proposal and its
/// adjudication key on the same canonical pair regardless of which stub each named first.
fn canonical_pair(from: MemoryId, to: MemoryId) -> (MemoryId, MemoryId) {
    if from <= to { (from, to) } else { (to, from) }
}

/// Parse a memory id (a ULID string, as the frontend serializes it) back into a [`MemoryId`].
fn parse_memory_id(id: &str) -> Result<MemoryId, JsError> {
    Ulid::from_string(id)
        .map(MemoryId)
        .map_err(|error| JsError::new(&format!("console: invalid memory id {id:?}: {error}")))
}
