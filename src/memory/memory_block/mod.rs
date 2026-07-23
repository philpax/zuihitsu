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
    engine::Engine,
    event::{
        Cardinality, ConversationRef, EventPayload, LinkSource, Teller, Visibility, Volatility,
    },
    graph::GraphError,
    ids::{ConversationId, EntryId, MemoryId, MemoryName, NamespacedMemoryName, TurnId},
    time::TemporalRef,
    vocabulary::{RelationName, TagName},
};

mod calendar;
mod effects;
mod error;
mod links;
mod reads;
mod resolve;
mod suggest;
mod tags;
mod writes;

pub use error::MemoryError;

/// How an entry argument to [`MemoryBlock::supersede`] or [`MemoryBlock::retract`] names its target.
/// An entry handle already carries the full id, so the Lua layer turns it into [`EntrySelector::Id`];
/// a bare string the agent typed — a full id, or a unique id prefix read off a rendered line — arrives
/// as [`EntrySelector::Ref`] and is resolved against the memory's class (see
/// [`MemoryBlock::resolve_entry_ref`]).
#[derive(Debug)]
pub enum EntrySelector {
    Id(EntryId),
    Ref(String),
}

/// What an `append` yielded once the dedup capture matrix ran: a fresh entry, or a corroboration of an
/// existing entry the write was found to duplicate. The Lua layer hands back the entry either way (a
/// corroboration returns the existing entry's handle) and surfaces the corroboration's note.
#[derive(Debug)]
pub enum AppendOutcome {
    /// A new content entry was recorded; its id.
    Appended(EntryId),
    /// The write corroborated an existing entry rather than recording a duplicate — see
    /// [`Corroboration`].
    Corroborated(Corroboration),
}

/// An entry an `append` or an explicit `attest` stood behind rather than recording anew: the existing
/// entry now attested, and the note the agent reads so the capture is never silent.
#[derive(Debug)]
pub struct Corroboration {
    pub entry: EntryId,
    pub note: String,
}

/// What a [`MemoryBlock::retract`] did. Under a conversation turn (platform authority) a retraction is
/// per-attester: when the fact is corroborated by other tellers, only the speaker's own account is
/// withdrawn and the entry stands; the Lua layer surfaces `Withdrawn`'s note into the agent's own
/// output so the partial withdrawal is never silent. A maintenance pass or the console (agent or
/// operator authority) always retracts the whole entry, as does a turn whose speaker is the fact's
/// sole teller.
#[derive(Debug)]
pub enum Retraction {
    /// The whole entry was tombstoned — the speaker was the fact's only teller, it was a
    /// public/attributed fact the speaker never attested, or the write ran under agent/operator
    /// authority (which retract the entry outright).
    Entry,
    /// Only the speaker's attestation was withdrawn; the fact stands, attested by the remaining
    /// tellers. The note names those visible to the present audience — never the hidden ones.
    Withdrawn { note: String },
}

impl From<EntryId> for EntrySelector {
    fn from(id: EntryId) -> EntrySelector {
        EntrySelector::Id(id)
    }
}

/// The fewest characters of an entry id that may address an entry by prefix. Short enough that the
/// rendered id's opening run is convenient to copy, long enough that a fragment is very unlikely to
/// collide with a second entry of the same memory — and a too-short fragment is refused outright
/// (see [`MemoryBlock::resolve_entry_ref`]) rather than resolving to whichever entry it happens to hit.
pub(super) const MIN_ENTRY_PREFIX: usize = 4;

/// Who is driving a block's writes. Operator authority is the console; it is the only path
/// permitted to edit `self`, and it authors its links as `Operator` rather than `Agent` (spec
/// §Imprint interview). Platform authority is an ordinary conversation turn. Agent authority
/// is a maintenance pass running off the hot path — consolidation, canonical-profile minting,
/// and link-redundant entry cleanup. It permits cross-teller supersede and free `same_as`
/// assertion (the powers these passes need) while still blocking `self` writes (`guard_self`
/// blocks all non-Operator authority). Narrower than a full self-evolution tier: no self-model
/// writes, no persona-source entries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Authority {
    Platform,
    Operator,
    Agent,
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
    /// The current conversation's [`Namespace::Context`] memory (the room content is told in), if
    /// any. Resolved independently of `told_in` (which carries the turn for attribution), so
    /// `current_context` can still return the room for the agent to append to.
    context_memory: Option<MemoryId>,
    /// The current conversation's location reference — a turn or a room (context memory) — where
    /// content is told in. Constructed from `turn_id` (when a turn is in context) and the
    /// conversation's context memory.
    told_in: Option<ConversationRef>,
    /// Whether `told_in` carries the `#confidential` tag — content here defaults private.
    confidential_context: bool,
    /// Who is present in the conversation — the set `memory.search` filters its hits against (spec
    /// §Visibility). Carried so the read path can reach it; writes do not use it.
    present_set: Vec<MemoryId>,
    /// The maximum character length of a single content entry, threaded from
    /// `MemorySettings::max_entry_chars`. An entry exceeding it is rejected before it is buffered.
    max_entry_chars: usize,
    buffer: Vec<EventPayload>,
    touched: BTreeSet<MemoryId>,
    aborted: Option<String>,
    /// A `turn.skip(reason)` signal — set by the `turn` module's `skip` function. Unlike `aborted`,
    /// a skip commits the block's buffered writes; it only suppresses the turn's reply.
    skip: Option<String>,
    /// Memories created this block whose seed entry (the `create(name, content)` argument) took the
    /// *unforced* write-time default and landed open (`Public`/`Attributed`). A later `exclude`
    /// append to such a memory in the same block is rejected as a teachable error: the open seed
    /// sitting beside the guard is the one plain copy that undoes it, caught at its point of
    /// failure. An explicitly classified seed is a deliberate choice and is never recorded here. An
    /// id left stale by an outer transaction rollback is unreachable (its create was rolled back
    /// with it), so the set is not snapshotted.
    open_default_seeds: BTreeSet<MemoryId>,
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
    /// Whether the entry has aged past usefulness *and nothing has replaced it*: its memory is `High`
    /// volatility, the fact is older than the staleness horizon (spec §Recency and volatility), and it
    /// is unsuperseded. A read marks it `[stale — no newer entry]` so the agent surfaces it as possibly
    /// out of date and reconfirms rather than hunting memory for a fresher version that does not exist.
    /// Always `false` for the default `Medium` and durable `Low` memories, so the marker is opt-in, and
    /// always `false` for a superseded entry (its successor is the newer version).
    pub stale: bool,
    /// The stated reason this entry was retracted, when it is a tombstone surfaced only by
    /// `mem:history()` — so a history read shows *why* a withdrawn fact was withdrawn beside it.
    /// `None` for a live or plainly-superseded entry.
    pub retracted_reason: Option<String>,
}

/// Which way a link runs relative to the memory it was read from. A class-traversing read orients
/// every edge against the queried identity, so the agent reads `mentors →` (this identity mentors)
/// apart from `mentors ←` (this identity is mentored), without reasoning about which stub holds it.
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
    /// for a link with no teller behind it (an operator-authored `same_as`) — the provenance a
    /// belief-bearing relation turns on, the same teller signal a content read carries.
    pub told_by: Option<String>,
    /// The far memory's representative occurrence — its most recent dated entry's `occurred_at`, or
    /// `None` when it holds no dated fact. Resolved at read time (like `other_name`) so a link to a
    /// dated event (a shipped decision, a scheduled meeting) carries *when* alongside the relation, and
    /// a neighborhood rendered from a hub keeps the spokes' dates without a second read. Not
    /// visibility-filtered, mirroring the link readers, which surface the agent's whole graph.
    pub occurred_at: Option<TemporalRef>,
}

/// A memory's whole record, assembled for `mem:details` — the one-render read that licenses "I don't
/// hold that" after a single look. It carries the memory's header (its current name, its description,
/// and any handles it used to go by), its live entries across the merged identity, every link out of
/// that identity in both directions, its applied tags, and its volatility. The Lua layer renders it to
/// one string, reusing the same entry and link rendering `mem:entries`/`mem:links` use, so the record
/// reads back exactly as those readers show their rows.
pub struct MemoryDetails {
    pub name: String,
    pub description: String,
    pub former_names: Vec<String>,
    pub entries: Vec<EntryRef>,
    pub links: Vec<LinkRef>,
    pub tags: Vec<TagName>,
    pub volatility: Volatility,
}

/// What a finished block yields to its caller for commit (or, on abort/error, to discard): the
/// buffered side effects, the memories touched (the lock set, for compaction's working-set), and the
/// abort reason if [`MemoryBlock::abort`] was called.
pub struct BlockEffects {
    pub events: Vec<EventPayload>,
    pub touched: Vec<MemoryId>,
    pub aborted: Option<String>,
    /// A `turn.skip(reason)` signal — the block's buffered writes should be committed, but the turn
    /// should end silently. Mirrors `aborted` in shape but commits instead of discards.
    pub skip: Option<String>,
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

/// The forced visibility an append or link opts selects once its `visibility` posture and `exclude`
/// list have been reconciled: one of the three named postures, or an `exclude` list naming the
/// parties a confidence is additionally withheld from (a [`Visibility::Exclude`]). An exclude *is* a
/// private posture, so it carries no separate `visibility`: [`reconcile_forced_visibility`] folds a
/// redundant `visibility = "private"` beside it into the exclude and rejects a genuinely
/// contradictory `public` or `attributed`, keeping the resolution paths
/// ([`MemoryBlock::resolve_visibility`], [`MemoryBlock::resolve_link_visibility`]) fed a single
/// already-reconciled choice.
pub(super) enum ForcedVisibility {
    Choice(VisibilityChoice),
    Exclude(BTreeSet<MemoryId>),
}

/// Reconcile an append's (or link's) `visibility` posture and `exclude` list into the single forced
/// visibility the write is recorded at, or a teachable failure. An exclude already fixes the posture
/// (private, additionally withheld from the named parties), so a `visibility = "private"` set beside
/// it is redundant but consistent and folds into the exclude — rejecting a write that means exactly
/// what it says would only cost a recovery round-trip. Pairing an exclude with `public` or
/// `attributed` contradicts the posture the exclude fixes, so that is a
/// [`MemoryError::VisibilityConflict`]; an `exclude` naming no one is a [`MemoryError::ExcludeEmpty`]
/// (a confidence for its teller alone is `visibility = "private"`, not an empty exclude), whatever
/// visibility rides beside it. With neither opt set the write takes its write-time default, so this
/// returns `None`.
pub(super) fn reconcile_forced_visibility(
    visibility: Option<VisibilityChoice>,
    exclude: Option<BTreeSet<MemoryId>>,
) -> Result<Option<ForcedVisibility>, MemoryError> {
    match (visibility, exclude) {
        (_, Some(ids)) if ids.is_empty() => Err(MemoryError::ExcludeEmpty),
        (Some(VisibilityChoice::Private) | None, Some(ids)) => {
            Ok(Some(ForcedVisibility::Exclude(ids)))
        }
        (Some(_), Some(_)) => Err(MemoryError::VisibilityConflict),
        (Some(choice), None) => Ok(Some(ForcedVisibility::Choice(choice))),
        (None, None) => Ok(None),
    }
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
/// the speaker's; `told_by` attributes it to a specific teller other than the current speaker (a
/// relayed claim's source), overriding both `by_agent` and the speaker default; `visibility` forces
/// the visibility instead of the write-time default; `occurred_at` records the real-world time the
/// entry is *about*, distinct from when it is recorded (spec §Time); `volatility` classifies how fast
/// the memory's facts age, set inline rather than via a separate `set_volatility` call; `exclude`
/// records the entry as a confidence additionally withheld whenever any named party is present (a
/// [`Visibility::Exclude`]), mutually exclusive with `visibility`. Deserialized from the Lua `opts`
/// table, except `occurred_at`, `told_by`, and `exclude`: those are resolved at the Lua boundary (a
/// bare date string or handle for `occurred_at`, a handle or name for `told_by`, a list of handles or
/// names for `exclude`) and set on the struct after, so they carry the resolved [`TemporalRef`],
/// [`Teller`], and memory ids rather than a raw Lua value serde cannot decode.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct AppendOptions {
    pub by_agent: bool,
    pub visibility: Option<VisibilityChoice>,
    #[serde(skip)]
    pub occurred_at: Option<TemporalRef>,
    pub volatility: Option<VolatilityChoice>,
    #[serde(skip)]
    pub told_by: Option<Teller>,
    #[serde(skip)]
    pub exclude: Option<BTreeSet<MemoryId>>,
    /// An entry the dedup scan skips — that entry only — so a re-append the agent has decided names a
    /// genuinely different fact records anew rather than being folded into the entry a near-duplicate
    /// check matched. Every other capture still fires. Resolved at the Lua boundary (an entry handle,
    /// id, or unique id prefix) and set after, so it carries a resolved [`EntrySelector`] rather than a
    /// raw Lua value.
    #[serde(skip)]
    pub distinct_from: Option<EntrySelector>,
}

/// The overrides a `links.create` call accepts: `visibility` forces the visibility instead of the
/// write-time default; `exclude` records the link as a confidence additionally withheld whenever any
/// named party is present (a [`Visibility::Exclude`]), mutually exclusive with `visibility`.
/// `visibility` deserializes from the Lua `opts` table; `exclude` is resolved at the Lua boundary (a
/// list of handles or names) and set after, carrying resolved memory ids rather than a raw Lua value.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct LinkOptions {
    pub visibility: Option<VisibilityChoice>,
    #[serde(skip)]
    pub exclude: Option<BTreeSet<MemoryId>>,
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

/// How far back `overdue` looks when called with no explicit window — a modest fortnight, long enough
/// to catch a reminder whose day slipped past over a break, short enough not to dredge up stale dates.
const DEFAULT_OVERDUE_DAYS: i64 = 14;

impl MemoryBlock {
    /// Open a block for `conversation`: resolve the context it writes in and whether that room is
    /// `#confidential`. `turn_id` is the current turn (for attribution), or `None` for the operator
    /// console or genesis paths that have no turn. Fails only on a graph read error (infrastructure),
    /// never on agent input.
    pub fn new(
        engine: Arc<Engine>,
        teller: Teller,
        authority: Authority,
        conversation: Option<ConversationId>,
        turn_id: Option<TurnId>,
        present_set: Vec<MemoryId>,
        max_entry_chars: usize,
    ) -> Result<MemoryBlock, GraphError> {
        let (told_in, context_memory, confidential_context, self_id, operator_id) = {
            let graph = engine.graph.lock();
            // The conversation is `None` for operator console paths (self-edit, retraction) that have
            // no meaningful conversation to attribute the write to — provenance is carried by
            // `EventSource::Operator` and `Authority::Operator` instead. Genesis likewise writes
            // `told_in: None` directly, bypassing the block entirely.
            let (context_memory, confidential_context, told_in) = match conversation {
                Some(conversation) => {
                    let context_memory = graph.context_for_conversation(conversation)?;
                    let confidential_context = match context_memory {
                        Some(context_id) => graph
                            .memory_by_id(context_id)?
                            .is_some_and(|context| context.tags.contains(&TagName::Confidential)),
                        None => false,
                    };
                    let told_in = Some(ConversationRef {
                        conversation,
                        turn: turn_id,
                    });
                    (context_memory, confidential_context, told_in)
                }
                None => (None, false, None),
            };
            let self_id = graph.self_memory()?.map(|memory| memory.id);
            let operator_id = graph
                .memory_by_name(NamespacedMemoryName::operator())?
                .map(|memory| memory.id);
            (
                told_in,
                context_memory,
                confidential_context,
                self_id,
                operator_id,
            )
        };
        Ok(MemoryBlock {
            engine,
            teller,
            authority,
            self_id,
            operator_id,
            context_memory,
            told_in,
            confidential_context,
            present_set,
            max_entry_chars,
            buffer: Vec::new(),
            touched: BTreeSet::new(),
            aborted: None,
            skip: None,
            open_default_seeds: BTreeSet::new(),
        })
    }
}

#[cfg(test)]
mod tests;
