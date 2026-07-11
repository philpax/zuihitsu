//! The console's WASM bridge.
//!
//! A [`Replica`] holds an event log and the graph it folds into, using `zuihitsu-core`'s real
//! materializer — the same projection the live agent runs (see `console/CONTRIBUTING.md`). The frontend
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
use sha2::{Digest, Sha256};
use ulid::Ulid;
use wasm_bindgen::prelude::*;
use zuihitsu_core::{
    brief::{BriefRequest, compose_traced},
    event::{Event, EventPayload, MergeProposalSource, RequestRecord},
    graph::Graph,
    ids::{MemoryId, MemoryName, Namespace, Seq},
    model::{Message, ToolChoice, ToolSpec},
    settings::BriefSettings,
    time::{MILLIS_PER_DAY, Timestamp},
};

/// A materializing read replica: an event log plus the graph state it folds into. The log is
/// retained so the graph can be re-folded to any earlier `Seq` for time-travel.
#[wasm_bindgen]
pub struct Replica {
    events: Vec<Event>,
    graph: Graph,
    head: Seq,
}

/// How many instances of a single recurring rule the agenda expands within its horizon — a bound so
/// a daily rule cannot flood the view (a weekly or monthly rule stays well under it over the
/// horizon).
const MAX_RECURRING_INSTANCES: usize = 20;

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
            let to_class = self.graph.class_id(to).map_err(graph_error)?;
            let merged = from_class.is_some() && from_class == to_class;
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
                // A stub is its class's primary when the class id resolves to itself.
                from_primary: from_class == Some(from),
                to_primary: to_class == Some(to),
                from_designated: self
                    .graph
                    .is_primary_designated(from)
                    .map_err(graph_error)?,
                to_designated: self.graph.is_primary_designated(to).map_err(graph_error)?,
            });
        }
        to_js(&out)
    }

    /// Verify every model call's recorded prompt against the digest stamped at send time: each
    /// call's request is reconstructed from the recorded deltas (base plus continuations, the same
    /// walk the frontend renders from), re-serialized, and hashed with the recorder's own code
    /// path. A `verified` call's displayed prompt provably matches the request that was sent; a
    /// `mismatch` means the reconstruction diverged and must not be trusted silently.
    #[wasm_bindgen(js_name = requestDigests)]
    pub fn request_digests(&self) -> Result<JsValue, JsError> {
        struct Group {
            system: String,
            messages: Vec<Message>,
            tools: Vec<ToolSpec>,
            tool_choice: ToolChoice,
            thinking: Option<bool>,
        }
        let mut groups: BTreeMap<String, Group> = BTreeMap::new();
        let mut checks: Vec<DigestCheck> = Vec::new();
        for event in &self.events {
            let EventPayload::ModelCalled {
                turn_id,
                phase,
                request_digest,
                request,
                ..
            } = &event.payload
            else {
                continue;
            };
            let key = format!("{} {phase:?}", turn_id.0);
            match request {
                Some(RequestRecord::Base {
                    system,
                    messages,
                    tools,
                    tool_choice,
                    thinking,
                    ..
                }) => {
                    groups.insert(
                        key.clone(),
                        Group {
                            system: system.clone(),
                            messages: messages.clone(),
                            tools: tools.clone(),
                            tool_choice: *tool_choice,
                            thinking: *thinking,
                        },
                    );
                }
                Some(RequestRecord::Continuation { appended_messages }) => {
                    match groups.get_mut(&key) {
                        Some(group) => group.messages.extend(appended_messages.iter().cloned()),
                        // An orphaned continuation (its base fell outside the log) reconstructs
                        // nothing.
                        None => {
                            checks.push(DigestCheck {
                                seq: event.seq.0,
                                status: "unrecorded",
                            });
                            continue;
                        }
                    }
                }
                None => {
                    checks.push(DigestCheck {
                        seq: event.seq.0,
                        status: "unrecorded",
                    });
                    continue;
                }
            }
            let group = groups.get(&key).expect("just inserted or extended");
            let view = RequestDigestView {
                system: &group.system,
                messages: &group.messages,
                tools: &group.tools,
                tool_choice: group.tool_choice,
                response_format: None,
                thinking: group.thinking,
            };
            let mut hasher = Sha256::new();
            hasher.update(serde_json::to_vec(&view).unwrap_or_default());
            let digest: String = hasher
                .finalize()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect();
            checks.push(DigestCheck {
                seq: event.seq.0,
                status: if digest == *request_digest {
                    "verified"
                } else if matches!(phase, zuihitsu_core::event::ModelPhase::Synthesis) {
                    // A structured synthesis call carries a `response_format` the record does not
                    // store, so its digest cannot be reproduced — unverifiable, not a mismatch.
                    "unverifiable"
                } else {
                    "mismatch"
                },
            });
        }
        to_js(&checks)
    }

    /// Re-derive a session's contextual brief and the trace of how it was composed — every memory the
    /// composer considered and, per entry, the visibility verdict and whether it reached the brief.
    /// The inputs are the session's present set (memory ids), its room's [`Namespace::Context`]
    /// memory (if any), its start time, and its recorded working set (from the `SessionStarted`
    /// payload; empty for sessions recorded before capture). The brief is composed against the graph
    /// at the current fold horizon, with the brief settings folded from the log at the same horizon —
    /// so a caller that folds to the session's seq re-derives exactly what the server composed.
    pub fn brief(
        &self,
        present_set: Vec<String>,
        context: Option<String>,
        now_ms: f64,
        working_set: Vec<String>,
    ) -> Result<JsValue, JsError> {
        let present = present_set
            .iter()
            .map(|id| parse_memory_id(id))
            .collect::<Result<Vec<_>, _>>()?;
        let current_context = match context {
            Some(id) => Some(parse_memory_id(&id)?),
            None => None,
        };
        let working = working_set
            .iter()
            .map(|id| parse_memory_id(id))
            .collect::<Result<Vec<_>, _>>()?;
        let request = BriefRequest {
            present_set: &present,
            current_context,
            working_set: &working,
            now: Timestamp::from_millis(now_ms as i64),
        };
        let trace = compose_traced(&self.graph, &self.brief_settings_at_fold(), &request)
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

    /// The brief settings in effect at the current fold horizon: the latest `ConfigSet` snapshot with
    /// `seq <= head`, defaults when none has been folded. Mirrors `Settings::from_store`'s fold — the
    /// replica holds a `Vec<Event>` rather than a `Store`, so the loop runs over the log directly.
    fn brief_settings_at_fold(&self) -> BriefSettings {
        let mut settings = BriefSettings::default();
        for event in self.events.iter().filter(|event| event.seq <= self.head) {
            if let EventPayload::ConfigSet {
                settings: logged, ..
            } = &event.payload
            {
                settings = logged.brief.clone();
            }
        }
        settings
    }
}

/// One call's digest verification result, keyed by the `ModelCalled` event's seq.
#[derive(Serialize)]
struct DigestCheck {
    seq: u64,
    status: &'static str,
}

/// Mirrors `zuihitsu::model::GenerateRequest`'s serialized shape exactly — same field names, order,
/// and inner types — so the digest computed here matches the recorder's `serde_json::to_vec` byte
/// for byte. The record does not carry `response_format`, so only requests without one (every Step
/// call) can verify; a drift between this mirror and the real struct surfaces as visible mismatch
/// verdicts in the console, never silently.
#[derive(Serialize)]
struct RequestDigestView<'a> {
    system: &'a str,
    messages: &'a [Message],
    tools: &'a [ToolSpec],
    tool_choice: ToolChoice,
    response_format: Option<serde_json::Value>,
    thinking: Option<bool>,
}

/// Serialize a value to a JS value with the JSON-compatible number policy (plain numbers, not
/// `BigInt`), so the result matches the ts-rs bindings the frontend is typed against.
pub(crate) fn to_js<T: Serialize>(value: &T) -> Result<JsValue, JsError> {
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
pub mod turn_ref;
pub mod types;

pub use types::{
    AgendaItem, ConversationDetail, MemoryDetail, MergeProposalView, MergeStatus, SessionSummary,
};

#[cfg(test)]
mod digest_tests {
    //! The digest mirror must serialize to exactly `GenerateRequest`'s bytes — the twin canary
    //! pinning the same literal lives beside that struct in the main crate.
    use super::RequestDigestView;
    use zuihitsu_core::model::ToolChoice;

    #[test]
    fn the_digest_view_serializes_with_the_generate_request_shape() {
        let view = RequestDigestView {
            system: "",
            messages: &[],
            tools: &[],
            tool_choice: ToolChoice::Auto,
            response_format: None,
            thinking: None,
        };
        assert_eq!(
            serde_json::to_string(&view).unwrap(),
            r#"{"system":"","messages":[],"tools":[],"tool_choice":"Auto","response_format":null,"thinking":null}"#
        );
    }
}
