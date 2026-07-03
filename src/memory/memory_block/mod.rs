//! The block transaction: the agent's memory-mutation surface as one opaque, invariant-enforcing
//! object.
//!
//! A [`MemoryBlock`] accumulates the side-effect events of a single Lua block — creates, appends,
//! links — committed or discarded atomically by the caller. It owns the buffer and the touched set,
//! resolves reads against the graph overlaid with its own pending writes (read-your-writes), and is
//! the one place the write invariants live: name uniqueness, registered relations, and the
//! write-time visibility default (including the `#confidential`-room firming). The Lua layer
//! ([`crate::agent::lua`]) is a thin wrapper over this — it translates script calls into method calls and
//! never touches the buffer, the events, or the visibility rules directly.

use std::{collections::BTreeSet, sync::Arc};

use serde::Deserialize;

use crate::{
    decay,
    engine::Engine,
    event::{Cardinality, EventPayload, LinkSource, Teller, Visibility, Volatility},
    graph::{EntryView, Graph, GraphError},
    ids::{ConversationId, EntryId, MemoryId, MemoryName, NamespacedMemoryName},
    time::TemporalRef,
    vocabulary::{RelationName, TagName},
};

use super::visibility::{default_visibility_named, subject_participant, visible};

mod calendar;
mod error;
mod links;
mod reads;
mod tags;
mod writes;

pub use error::MemoryError;

/// Who is driving a block's writes. Operator authority is the console; it is the only path
/// permitted to edit `self`, and it authors its links as `Operator` rather than `Agent` (spec
/// §Imprint interview). Platform authority is an ordinary conversation turn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Authority {
    Platform,
    Operator,
}

/// One block's in-progress memory mutations. Built fresh per block, mutated through its operations,
/// and consumed by [`MemoryBlock::into_effects`] to commit or discard.
///
/// The graph and clock are reached through a shared [`Engine`] handle rather than borrows, so the
/// block (and the Lua functions that drive it) is `'static` — the Lua API now runs through
/// `Lua::create_function` and `eval_async` rather than a scope, and a scoped borrow could not survive
/// that. The graph is locked transiently per read; no guard is ever held across an `.await`.
pub struct MemoryBlock {
    engine: Arc<Engine>,
    /// The turn's teller, attributed to content written this block unless an append opts out.
    teller: Teller,
    /// Whether this block runs under platform or operator authority — gates `self` writes and the
    /// link source.
    authority: Authority,
    /// The `self` memory's id, resolved once at open so every write path can guard it with a cheap id
    /// compare. `None` only before genesis seeds `self`.
    self_id: Option<MemoryId>,
    /// The `person/operator` anchor's id, resolved once at open so content writes to it can be guarded
    /// with a cheap id compare. `None` before the first imprint mints it (or in a non-operator
    /// instance). Content belongs on the operator's real profile, not this provisional anchor.
    operator_id: Option<MemoryId>,
    /// The current conversation's `context/*` memory (where content is told in), if any.
    told_in: Option<MemoryId>,
    /// Whether `told_in` carries the `#confidential` tag — content here defaults private.
    confidential_context: bool,
    /// Who is present in the conversation — the set `memory.search` filters its hits against (spec
    /// §Visibility). Carried so the read path can reach it; writes do not use it.
    present_set: Vec<MemoryId>,
    buffer: Vec<EventPayload>,
    touched: BTreeSet<MemoryId>,
    aborted: Option<String>,
}

/// An addressable content entry handed back to the agent: its stable [`EntryId`] and its text. The
/// id lets the agent pass an entry to [`MemoryBlock::supersede`]; the Lua layer renders it as its
/// text so reading stays ergonomic.
pub struct EntryRef {
    pub entry_id: EntryId,
    pub text: String,
    /// How widely the entry may surface — so a read renders it self-describingly and the agent sees at
    /// a glance whether a fact is a confidence to hold (`PrivateToTeller`/`Exclude`) or freely shareable.
    pub visibility: Visibility,
    /// Who the entry is attributed to, resolved to a readable label ("person/erin", "you" for the
    /// agent's own note) — so a read shows where a fact came from, which is what tells the agent whose
    /// confidence it is.
    pub teller: String,
    /// Whether the entry is under an unresolved belief arbitration — a fact the agent recorded as
    /// contested and should surface as such rather than assert as settled. Lets a read advertise the
    /// dispute so the agent honors it when answering, instead of confidently picking one account.
    pub disputed: bool,
    /// When the fact occurs, if dated — so a read shows the date inline rather than leaving it in a
    /// structured field the agent must inspect or search for separately. `None` for an undated note.
    pub occurred_at: Option<TemporalRef>,
    /// Whether the entry's content was withheld from this read because the present audience is not
    /// cleared to see it (the same visibility predicate search applies — a confidence whose teller is
    /// absent, or guarded by an `Exclude`/subject rule). When set, `text` is a stub naming that
    /// something was confided, not the confidence itself: the agent learns a private fact exists, and
    /// by whom and when, without being handed content it must not relay to who is present. Only ever
    /// set on a direct read with an audience present; a solo read sees everything.
    pub withheld: bool,
    /// Whether the entry has aged past usefulness: its memory is `High` volatility and the fact is
    /// older than the staleness horizon (spec §Recency and volatility). A read marks it `[stale]` so the agent
    /// surfaces it as possibly out of date rather than asserting it as current. Always `false` for the
    /// default `Medium` and durable `Low` memories, so the marker is opt-in.
    pub stale: bool,
}

/// Which way a link runs relative to the memory it was read from. A class-traversing read orients
/// every edge against the queried identity, so the agent reads `mentor_of →` (this identity mentors)
/// apart from `mentor_of ←` (this identity is mentored), without reasoning about which stub holds it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkDirection {
    /// The queried identity is the edge's source: it points outward, at `other`.
    Outgoing,
    /// The queried identity is the edge's target: `other` points at it.
    Incoming,
}

/// A link handed back to the agent by a link reader (`mem:outgoing`/`incoming`/`links`): the related
/// memory, the relation under which they are linked, which way the edge runs relative to the queried
/// identity, and the link's provenance. The far endpoint's name is resolved at read time so the Lua
/// layer can render the link self-describingly (`relation → name`) without a second lookup.
pub struct LinkRef {
    pub relation: RelationName,
    /// The memory at the other end of the edge — never a member of the queried `same_as` class, since
    /// a reader surfaces relationships pointing out of the identity, not the `same_as` plumbing within it.
    pub other: MemoryId,
    pub other_name: MemoryName,
    pub direction: LinkDirection,
    pub source: LinkSource,
    /// Who asserted the relationship, resolved to a readable label ("person/erin", "you"), or `None`
    /// for a link with no teller behind it (the adjudicated `same_as`) — the provenance a belief-bearing
    /// relation turns on, the same teller signal a content read carries.
    pub told_by: Option<String>,
}

/// What a finished block yields to its caller for commit (or, on abort/error, to discard): the
/// buffered side effects, the memories touched (the lock set, for compaction's working-set), and the
/// abort reason if [`MemoryBlock::abort`] was called.
pub struct BlockEffects {
    pub events: Vec<EventPayload>,
    pub touched: Vec<MemoryId>,
    pub aborted: Option<String>,
}

/// The forced visibility a `visibility = "public" | "attributed" | "private"` append opt selects,
/// deserialized from the Lua opts table.
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VisibilityChoice {
    Public,
    Attributed,
    Private,
}

/// The forced volatility a `volatility = "low" | "medium" | "high"` opt selects — how fast the
/// memory's facts age (spec §Recency and volatility). Lets an append classify the memory inline,
/// rather than a separate `set_volatility` call.
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VolatilityChoice {
    Low,
    Medium,
    High,
}

impl VolatilityChoice {
    pub(super) fn into_volatility(self) -> Volatility {
        match self {
            VolatilityChoice::Low => Volatility::Low,
            VolatilityChoice::Medium => Volatility::Medium,
            VolatilityChoice::High => Volatility::High,
        }
    }
}

/// The overrides an append accepts: `by_agent` records it as the agent's own observation rather than
/// the speaker's; `visibility` forces the visibility instead of the write-time default; `occurred_at`
/// records the real-world time the entry is *about*, distinct from when it is recorded (spec §Time);
/// `volatility` classifies how fast the memory's facts age, set inline rather than via a separate
/// `set_volatility` call. Deserialized straight from the Lua `opts` table — `occurred_at` is a tagged
/// table (see [`TemporalRef`]).
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct AppendOptions {
    pub by_agent: bool,
    pub visibility: Option<VisibilityChoice>,
    pub occurred_at: Option<TemporalRef>,
    pub volatility: Option<VolatilityChoice>,
}

/// A link relation to register, deserialized straight from the `links.register` table. Cardinalities
/// arrive as the lowercase strings the spec advertises (`"one"` / `"many"`) and are parsed at the
/// block boundary; `symmetric` and `reflexive` default to `false`; `description` defaults to empty
/// for a relation registered without one (the seed relations always carry one).
#[derive(Debug, Deserialize)]
pub struct RelationSpec {
    pub name: String,
    pub inverse: String,
    pub from_card: String,
    pub to_card: String,
    #[serde(default)]
    pub symmetric: bool,
    #[serde(default)]
    pub reflexive: bool,
    #[serde(default)]
    pub description: String,
}

/// Parse a cardinality from the lowercase string `links.register` advertises (`"one"` / `"many"`),
/// case-insensitively. A `Cardinality` serializes as `One`/`Many` on the wire, but the agent-facing
/// API speaks lowercase, so the two are reconciled here rather than by widening the wire format.
pub(super) fn parse_cardinality(value: &str) -> Result<Cardinality, MemoryError> {
    value
        .parse()
        .map_err(|()| MemoryError::BadCardinality(value.to_owned()))
}

/// The text a withheld entry carries in place of its content (see [`EntryRef::withheld`]). It names
/// only that something was confided — the date and teller ride the entry's own marker — so the agent
/// can acknowledge a confidence exists and decline to share it, without ever holding the words.
const WITHHELD_STUB: &str = "(withheld — a confidence not for the present audience)";

/// Memories with a concrete occurrence within this many days of now, when `upcoming` is called with
/// no explicit window.
const DEFAULT_UPCOMING_DAYS: i64 = 7;

impl MemoryBlock {
    /// Open a block for `conversation`: resolve the context it writes in and whether that room is
    /// `#confidential`. Fails only on a graph read error (infrastructure), never on agent input.
    pub fn new(
        engine: Arc<Engine>,
        teller: Teller,
        authority: Authority,
        conversation: ConversationId,
        present_set: Vec<MemoryId>,
    ) -> Result<MemoryBlock, GraphError> {
        let (told_in, confidential_context, self_id, operator_id) = {
            let graph = engine.graph.lock();
            let told_in = graph.context_for_conversation(conversation)?;
            let confidential_context = match told_in {
                Some(context_id) => graph
                    .memory_by_id(context_id)?
                    .is_some_and(|context| context.tags.contains(&TagName::Confidential)),
                None => false,
            };
            let self_id = graph.self_memory()?.map(|memory| memory.id);
            let operator_id = graph
                .memory_by_name(NamespacedMemoryName::operator())?
                .map(|memory| memory.id);
            (told_in, confidential_context, self_id, operator_id)
        };
        Ok(MemoryBlock {
            engine,
            teller,
            authority,
            self_id,
            operator_id,
            told_in,
            confidential_context,
            present_set,
            buffer: Vec::new(),
            touched: BTreeSet::new(),
            aborted: None,
        })
    }

    /// A handle to the shared backends and the present set this block runs under — the inputs the
    /// async `memory.search` needs (the embedder, vector index, graph, clock, settings, and visibility
    /// set). Returned together so the Lua layer can embed and search without holding the block lock.
    pub fn retrieval_handle(&self) -> (Arc<Engine>, Vec<MemoryId>) {
        (self.engine.clone(), self.present_set.clone())
    }

    /// The backends and present set the `convo.turn` link resolver reads under — the running engine
    /// (for the event store and graph) and who is present in this conversation, so the resolver can
    /// apply the audience rule: a turn resolves iff everyone present here was in that moment's audience
    /// (spec §Transcripts). Returned together so the Lua layer can resolve without holding the block
    /// lock.
    pub fn turn_resolution_handle(&self) -> (Arc<Engine>, Vec<MemoryId>) {
        (self.engine.clone(), self.present_set.clone())
    }

    /// The current conversation's context memory, or `None` — touches it so it enters the lock set.
    pub fn current_context(&mut self) -> Option<MemoryId> {
        if let Some(id) = self.told_in {
            self.touched.insert(id);
        }
        self.told_in
    }

    /// Discard everything this block buffered and end it, recording `reason` as the terminal cause.
    pub fn abort(&mut self, reason: Option<String>) {
        self.aborted = Some(reason.unwrap_or_default());
    }

    /// Consume the block for commit: the buffered events, the touched lock set, and any abort reason.
    pub fn into_effects(self) -> BlockEffects {
        BlockEffects {
            events: self.buffer,
            touched: self.touched.into_iter().collect(),
            aborted: self.aborted,
        }
    }

    /// Drain the block's effects without consuming it. The block now lives behind a shared
    /// `Arc<Mutex<…>>` (so the Lua functions can hold `'static` handles to it), which cannot be
    /// `try_unwrap`ped while those function references survive in the VM, so the caller reclaims the
    /// effects through the lock instead. Leaves the block empty.
    pub fn take_effects(&mut self) -> BlockEffects {
        BlockEffects {
            events: std::mem::take(&mut self.buffer),
            touched: std::mem::take(&mut self.touched).into_iter().collect(),
            aborted: self.aborted.take(),
        }
    }

    /// Run a compound operation as a transaction: if `body` returns `Err`, discard every event it
    /// buffered so a failure partway through a multi-event operation leaves no orphaned writes, then
    /// propagate the error. The touched set is left intact — reads within the operation genuinely
    /// touched those memories, and a rolled-back write's target was still interacted with. A
    /// single-event operation needs no transaction: its one check-then-buffer is already atomic,
    /// since the check precedes the (infallible) buffer push. Used by [`MemoryBlock::revise`] and
    /// [`MemoryBlock::create_with_opts`].
    pub(super) fn transaction<R>(
        &mut self,
        body: impl FnOnce(&mut Self) -> Result<R, MemoryError>,
    ) -> Result<R, MemoryError> {
        let savepoint = self.buffer.len();
        match body(self) {
            Ok(value) => Ok(value),
            Err(error) => {
                self.buffer.truncate(savepoint);
                Err(error)
            }
        }
    }

    /// Reject a platform-authority write that touches `self`. The console (operator authority)
    /// is the only path permitted to edit `self`, so the self model cannot be forged from a
    /// conversation (spec §Imprint interview). `create("self")` needs no guard — it is already blocked
    /// by `NameExists`, since `self` is seeded at genesis.
    pub(super) fn guard_self(&self, id: MemoryId) -> Result<(), MemoryError> {
        if self.authority == Authority::Platform && Some(id) == self.self_id {
            return Err(MemoryError::SelfWriteForbidden);
        }
        Ok(())
    }

    /// Reject a content write to the `person/operator` anchor (under any authority). The anchor holds
    /// no content of its own — facts about the operator belong on their real `person/<name>` profile,
    /// which is merged into it — so it stays a pure merge target. The merge (`same_as`) and `created_by`
    /// links to it are not content, so they are unaffected.
    pub(super) fn guard_operator(&self, id: MemoryId) -> Result<(), MemoryError> {
        if Some(id) == self.operator_id {
            return Err(MemoryError::OperatorWriteForbidden);
        }
        Ok(())
    }

    /// The visibility a content entry is written at, or a teachable failure. An explicit choice is
    /// honored verbatim. With none: a `#confidential` room firms everything private; otherwise an
    /// agent-authored entry about a *person* (a subject-bearing memory) has no protective default —
    /// the participant-aside mechanism keys on a participant teller, not the agent, so silently
    /// defaulting to public is how a re-recorded confidence leaks — and must be classified. Any other
    /// write (a participant teller, or a non-subject memory like `self`/`topic/*`) takes the
    /// namespace/subject default.
    pub(super) fn resolve_visibility(
        &self,
        name: Option<&str>,
        id: MemoryId,
        told_by: &Teller,
        explicit: Option<VisibilityChoice>,
    ) -> Result<Visibility, MemoryError> {
        if let Some(choice) = explicit {
            return Ok(match choice {
                VisibilityChoice::Public => Visibility::Public,
                VisibilityChoice::Attributed => Visibility::Attributed,
                VisibilityChoice::Private => Visibility::PrivateToTeller,
            });
        }
        if self.confidential_context {
            return Ok(Visibility::PrivateToTeller);
        }
        let about_a_person = name.is_some_and(|name| subject_participant(name, id).is_some());
        if matches!(told_by, Teller::Agent) && about_a_person {
            return Err(MemoryError::VisibilityRequired);
        }
        Ok(match name {
            Some(name) => default_visibility_named(name, id, told_by),
            None => Visibility::Public,
        })
    }

    /// Record `id` and its `same_as` class as touched (a traversing read locks the whole class), and
    /// return the class as a set for membership tests against the pending buffer.
    pub(super) fn touch_class(
        &mut self,
        id: MemoryId,
        members: Vec<MemoryId>,
    ) -> BTreeSet<MemoryId> {
        self.touched.insert(id);
        let mut set = BTreeSet::new();
        for member in members {
            self.touched.insert(member);
            set.insert(member);
        }
        set.insert(id);
        set
    }

    /// The entries this block has superseded but not yet committed — applied to the live reads so a
    /// correction's effect is visible within the block (read-your-writes).
    pub(super) fn pending_superseded(&self) -> BTreeSet<EntryId> {
        self.buffer
            .iter()
            .filter_map(|event| match event {
                EventPayload::MemorySuperseded { entry, .. } => Some(*entry),
                _ => None,
            })
            .collect()
    }

    /// This block's pending content appends to any member of `members`, as entry refs, skipping any in
    /// `exclude` — the read-your-writes tail of a live or history entry read.
    pub(super) fn pending_entries(
        &self,
        members: &BTreeSet<MemoryId>,
        exclude: &BTreeSet<EntryId>,
    ) -> Vec<EntryRef> {
        self.buffer
            .iter()
            .filter_map(|event| match event {
                EventPayload::MemoryContentAppended {
                    id,
                    entry_id,
                    text,
                    told_by,
                    visibility,
                    occurred_at,
                    ..
                } if members.contains(id) && !exclude.contains(entry_id) => Some(EntryRef {
                    entry_id: *entry_id,
                    text: text.clone(),
                    visibility: visibility.clone(),
                    teller: self.teller_label(told_by),
                    disputed: false,
                    occurred_at: occurred_at.clone(),
                    withheld: false,
                    stale: false,
                }),
                _ => None,
            })
            .collect()
    }

    /// Project an [`EntryView`] into an [`EntryRef`], resolving its teller to a readable label,
    /// marking it disputed when it is in the memory's set of unresolved-arbitration competing entries,
    /// and — when `withheld` — replacing its content with a stub so the confidence is not handed to a
    /// read whose present audience is not cleared to see it (see [`EntryRef::withheld`]).
    pub(super) fn entry_ref(
        &self,
        view: EntryView,
        disputed: &BTreeSet<EntryId>,
        withheld: bool,
        stale: bool,
    ) -> EntryRef {
        EntryRef {
            disputed: disputed.contains(&view.entry_id),
            entry_id: view.entry_id,
            text: if withheld {
                WITHHELD_STUB.to_owned()
            } else {
                view.text
            },
            visibility: view.visibility,
            teller: self.teller_label(&view.told_by),
            occurred_at: view.occurred_at,
            withheld,
            stale,
        }
    }

    /// Annotate each entry of a direct read with whether it is `withheld` and whether it is `stale`.
    ///
    /// *Withheld* applies the same [`visible`] predicate search does (resolving identity over the
    /// `same_as` class). Two deliberate carve-outs keep the agent's reach over its own memory intact:
    /// with no one present — a solo flush or maintenance pass — nothing is withheld; and the audience
    /// check ignores supersession (probing with `superseded_by` cleared), so `history` still shows a
    /// superseded entry yet still withholds one that was a confidence not for who is present.
    ///
    /// *Stale* is independent of who is present — it is a fact about the entry's age on a `High`
    /// volatility memory (spec §Recency and volatility) — so it is computed for every read, audience or not.
    pub(super) fn annotate(
        &self,
        graph: &Graph,
        id: MemoryId,
        entries: Vec<EntryView>,
    ) -> Result<Vec<(EntryView, bool, bool)>, MemoryError> {
        let now = self.now();
        let memory = graph.memory_by_id(id)?;
        let volatility = memory
            .as_ref()
            .map(|memory| memory.volatility)
            .unwrap_or_default();
        let audience = !self.present_set.is_empty();
        let class_of = |mid| graph.class_id(mid).map(|class| class.unwrap_or(mid));
        entries
            .into_iter()
            .map(|entry| {
                let effective = entry.occurred_sort.unwrap_or(entry.asserted_at);
                let stale = decay::is_stale(volatility, effective, now);
                let withheld = match (audience, &memory) {
                    (true, Some(memory)) => {
                        let mut probe = entry.clone();
                        probe.superseded_by = None;
                        !visible(&probe, memory, &self.present_set, &class_of)?
                    }
                    _ => false,
                };
                Ok((entry, withheld, stale))
            })
            .collect()
    }

    /// A readable label for who an entry is attributed to: the participant's canonical handle, `you`
    /// for the agent's own observations, or `genesis` for seeded content.
    pub(super) fn teller_label(&self, teller: &Teller) -> String {
        match teller {
            Teller::Participant(id) => self
                .resolve_name(*id)
                .ok()
                .flatten()
                .map(|name| name.as_str().to_owned())
                .unwrap_or_else(|| "someone".to_owned()),
            Teller::Agent => "you".to_owned(),
            Teller::Bootstrap => "genesis".to_owned(),
        }
    }

    /// The live entry ids of `id`'s `same_as` class: committed-live (the graph already excludes
    /// committed supersessions) plus this block's pending appends, minus what it has superseded —
    /// the set [`MemoryBlock::supersede`] validates its arguments against.
    pub(super) fn live_class_entry_ids(
        &self,
        id: MemoryId,
    ) -> Result<BTreeSet<EntryId>, MemoryError> {
        let (members, committed) = {
            let graph = self.engine.graph.lock();
            (graph.class_members(id)?, graph.class_entries(id)?)
        };
        let members: BTreeSet<MemoryId> = members.into_iter().chain([id]).collect();
        let pending_superseded = self.pending_superseded();
        let mut ids: BTreeSet<EntryId> = committed
            .into_iter()
            .map(|entry| entry.entry_id)
            .filter(|entry_id| !pending_superseded.contains(entry_id))
            .collect();
        for entry in self.pending_entries(&members, &pending_superseded) {
            ids.insert(entry.entry_id);
        }
        Ok(ids)
    }

    /// Buffer a content entry and touch its memory, returning the minted entry id (so a write can be
    /// handed back to the agent as an addressable entry — see [`MemoryBlock::append`]).
    pub(super) fn push_content(
        &mut self,
        id: MemoryId,
        text: String,
        told_by: Teller,
        visibility: Visibility,
        occurred_at: Option<TemporalRef>,
    ) -> EntryId {
        let entry_id = EntryId::generate();
        self.touched.insert(id);
        self.buffer.push(EventPayload::MemoryContentAppended {
            id,
            entry_id,
            asserted_at: self.engine.clock.now(),
            occurred_at,
            text,
            told_by,
            told_in: self.told_in,
            visibility,
        });
        entry_id
    }

    /// Resolve a name to a memory id, consulting this block's pending creates/renames before the
    /// graph (read-your-writes).
    pub(super) fn resolve(&self, name: &str) -> Result<Option<MemoryId>, GraphError> {
        for event in &self.buffer {
            match event {
                EventPayload::MemoryCreated { id, name: created } if created.as_str() == name => {
                    return Ok(Some(*id));
                }
                EventPayload::MemoryRenamed { id, new_name, .. } if new_name.as_str() == name => {
                    return Ok(Some(*id));
                }
                _ => {}
            }
        }
        Ok(self
            .engine
            .graph
            .lock()
            .memory_by_name(MemoryName::new(name))?
            .map(|memory| memory.id))
    }

    /// A readable field of a memory by id — `name` or `description` — backing the handle metatable's
    /// lazy `handle.name` / `handle.description` accessors, so a memory handle minted from only an id (a
    /// calendar or link result) still reads its name. `name` honors this block's pending creates;
    /// `description` is graph-only (a just-created memory has none synthesized yet). An unknown field is
    /// `None`, so the metatable falls through to its methods.
    pub(crate) fn handle_field(
        &self,
        id: MemoryId,
        field: &str,
    ) -> Result<Option<String>, MemoryError> {
        match field {
            "name" => Ok(self.resolve_name(id)?.map(|name| name.as_str().to_owned())),
            "description" => Ok(self
                .engine
                .graph
                .lock()
                .memory_by_id(id)?
                .map(|memory| memory.description)),
            _ => Ok(None),
        }
    }

    /// Resolve a memory's name, honoring a pending `MemoryCreated` not yet projected — so a handle to a
    /// memory created this block reads its name (and the teller label for an entry attributed within the
    /// same block).
    pub(super) fn resolve_name(&self, id: MemoryId) -> Result<Option<MemoryName>, GraphError> {
        let pending = self.buffer.iter().find_map(|event| match event {
            EventPayload::MemoryCreated { id: created, name } if *created == id => {
                Some(name.clone())
            }
            _ => None,
        });
        match pending {
            Some(name) => Ok(Some(name)),
            None => Ok(self
                .engine
                .graph
                .lock()
                .memory_by_id(id)?
                .map(|memory| memory.name)),
        }
    }
}

#[cfg(test)]
mod tests;
