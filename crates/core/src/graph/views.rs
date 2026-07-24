//! The graph's read-model view types: the shapes the query methods return and the wire exports
//! the console's replica consumes. Pure data with small accessors — the queries that produce them
//! live in the sub-domain modules, and the [`Graph`](crate::graph::Graph) handle itself stays in
//! the module root.

use serde::{Deserialize, Serialize};

use crate::{
    event::{
        Cardinality, ConversationRef, EventSource, LinkSource, Teller, Visibility, Volatility,
    },
    ids::{ConversationId, ConversationLocator, EntryId, MemoryId, MemoryName, Seq, SessionId},
    time::{TemporalRef, Timestamp},
    vocabulary::{RelationName, TagName},
};

/// A memory as projected, with its applied tags. Soft-deleted memories are never returned here.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct MemoryView {
    pub id: MemoryId,
    pub name: MemoryName,
    pub description: String,
    pub volatility: Volatility,
    pub created_at: Timestamp,
    pub tags: Vec<TagName>,
}

/// A live entry that carries a recurrence rule, with the memory it belongs to and the rule text —
/// the projection behind the console's per-memory recurring list, so the operator reads which rooms
/// carry recurring occurrences from the graph rather than a re-fold of the log (see
/// [`Graph::recurring_entries`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RecurringEntry {
    pub memory: MemoryId,
    pub text: String,
    pub rrule: String,
}

/// One teller's endorsement of an entry's fact (spec §Visibility → attestations). An entry carries a
/// set of these rather than a single teller: the founding attestation, derived at materialization
/// from the entry's own `MemoryContentAppended`, plus any [`EventPayload::EntryAttested`] a further
/// teller added. Each attestation is evaluated against the audience by its own `posture` and
/// `teller`, and the visible subset is the chip rule's engine — a hidden attestation (one whose
/// posture does not pass for the present audience) is absent from that subset even when the fact
/// itself renders.
///
/// [`EventPayload::EntryAttested`]: crate::event::EventPayload::EntryAttested
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AttestationView {
    /// The teller who stands behind the fact — resolved for the read-time predicate and the marker.
    pub teller: Teller,
    /// The conversation reference (turn or room) this attestation was asserted in, mirroring the
    /// entry's own `told_in`.
    pub told_in: Option<ConversationRef>,
    /// When the attester recorded the endorsement.
    pub asserted_at: Timestamp,
    /// The attester's own audience posture, at or narrower than the entry's founding posture (the
    /// audience-widening invariant, enforced by the write path).
    pub posture: Visibility,
    /// The attester's own wording when it differed from the entry text, kept for history and the
    /// console only; `None` when the attester added no distinct phrasing.
    pub phrasing: Option<String>,
    /// The retired entry a consolidation carried this attestation from, or `None` for an attestation
    /// asserted directly on this entry.
    pub source_entry: Option<EntryId>,
    /// The stated reason this attestation was withdrawn, on a history read; `None` for a live
    /// attestation (the live entry reads carry only live attestations).
    pub retracted_reason: Option<String>,
}

impl AttestationView {
    /// The founding attestation an entry carries by construction: the endorsement its own
    /// `MemoryContentAppended` recorded. Derived from the entry's `told_by`/`told_in`/`asserted_at`/
    /// `visibility`, so a plain single-teller entry has exactly this one attestation and the
    /// visibility predicate over it is bit-identical to reasoning over the entry's own fields.
    pub fn founding(
        teller: Teller,
        told_in: Option<ConversationRef>,
        asserted_at: Timestamp,
        posture: Visibility,
    ) -> AttestationView {
        AttestationView {
            teller,
            told_in,
            asserted_at,
            posture,
            phrasing: None,
            source_entry: None,
            retracted_reason: None,
        }
    }
}

/// A content entry as projected, ordered within its memory by commit order. `occurred_sort` is the
/// denormalized representative instant of the entry's `occurred_at` (spec §Time), or `None` when the
/// entry carries no occurrence (or only a `Recurring` one); recency ranking reads it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct EntryView {
    pub entry_id: EntryId,
    pub asserted_at: Timestamp,
    pub occurred_sort: Option<Timestamp>,
    /// The entry's typed occurrence — when the fact happens — or `None` if undated. Carried alongside
    /// the flattened `occurred_sort` so a read can render the date faithfully (a recurrence or range,
    /// not just its sort instant), letting the agent see *when* on read instead of inspecting a
    /// structured field or searching for a date that lives outside the entry text.
    pub occurred_at: Option<TemporalRef>,
    /// Whether `occurred_at` was authored at append — the agent stamped it — rather than inferred
    /// later by the turn-end temporal extraction. Authored is ground truth; extracted is inference,
    /// so a representative-date projection prefers an authored occurrence over an extracted one, and a
    /// guessed date never shadows a stated one. `false` for an undated entry (it has no occurrence to
    /// classify) and for one whose occurrence was resolved by extraction.
    pub occurred_authored: bool,
    pub text: String,
    pub told_by: Teller,
    pub told_in: Option<ConversationRef>,
    pub visibility: Visibility,
    /// The entry that replaced this one, when it has been superseded (spec §Visibility → superseded
    /// entries are not live). `None` for a live entry. Live reads exclude superseded entries in SQL;
    /// this field surfaces on the history reads that deliberately include them. A retracted entry
    /// carries its *own* id here — the self-referential tombstone that makes every `superseded_by IS
    /// NULL` live filter hide it — so a consumer distinguishes a retraction from a supersession by
    /// `retracted_reason`, never by reading this as a successor.
    pub superseded_by: Option<EntryId>,
    /// The stated reason this entry was retracted, or `None` for a live or plainly-superseded entry.
    /// Present only on the history reads (a retraction drops from every live surface); the surfaces
    /// that show a retracted entry render this reason beside it.
    pub retracted_reason: Option<String>,
    /// Where this entry came from, derived from the recording event's [`EventSource`]. The one
    /// distinction the projection preserves is a connector-maintained entry (a participant's
    /// username, display name, or nickname projected and owned by a platform connector) versus an
    /// ordinary recorded one, since the maintenance cleanup passes must never mutate a
    /// connector-owned entry — the connector may supersede or retract it at any time.
    pub origin: EntryOrigin,
    /// The entry's live attestations — the set of tellers standing behind its fact — founding first,
    /// then by commit order. Every entry carries at least the founding attestation; a further teller's
    /// [`EventPayload::EntryAttested`] adds another. The visibility predicate reasons over this set
    /// (the widest passing verdict), and the visible subset is the chip rule's engine. Populated by
    /// the entry reads as a batched fetch; an entry built without it (a hand-built view) is read by
    /// the predicate as its founding attestation alone, so the two paths agree for a singleton.
    ///
    /// [`EventPayload::EntryAttested`]: crate::event::EventPayload::EntryAttested
    pub attestations: Vec<AttestationView>,
}

/// Where a content entry came from, projected from the recording event's [`EventSource`]. Kept
/// deliberately coarse: the only distinction a reader (and the cleanup passes) needs is whether the
/// entry is maintained by a platform connector or was recorded by the agent, operator, orchestration,
/// or genesis. A connector-owned entry is excluded from every autonomous cleanup pass, since the
/// connector holds its id and supersedes or retracts it as the platform-side account changes.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum EntryOrigin {
    /// Recorded by the agent, operator, orchestration, or genesis — everything that is not a platform
    /// connector. The default, and the origin of the overwhelming majority of entries.
    #[default]
    Recorded,
    /// Projected and maintained by a platform connector; the string names the platform it serves
    /// (mirroring [`EventSource::PlatformConnector`]). The cleanup passes never mutate such an entry.
    PlatformConnector(String),
}

impl EntryOrigin {
    /// The origin an entry recorded under `source` carries. Only a connector source is distinguished;
    /// every other source folds to [`EntryOrigin::Recorded`].
    pub fn from_source(source: &EventSource) -> EntryOrigin {
        match source {
            EventSource::PlatformConnector(platform) => {
                EntryOrigin::PlatformConnector(platform.clone())
            }
            _ => EntryOrigin::Recorded,
        }
    }

    /// Whether this entry is maintained by a platform connector — the case every cleanup pass excludes.
    pub fn is_connector(&self) -> bool {
        matches!(self, EntryOrigin::PlatformConnector(_))
    }
}

/// A registered relation as projected.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RelationView {
    pub name: RelationName,
    pub inverse: RelationName,
    pub from_card: Cardinality,
    pub to_card: Cardinality,
    pub symmetric: bool,
    pub reflexive: bool,
    /// The relation's one-line purpose, surfaced in the prompt and `links.list`/`get`.
    pub description: String,
}

/// A tag in the vocabulary as projected: its name, its one-line purpose, and how many live memories
/// carry it. Backs `tags.list` and the system prompt's tag-vocabulary block.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct TagVocabularyEntry {
    pub name: TagName,
    pub description: String,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub count: usize,
}

/// The fields the link visibility predicate reasons over, extracted from any link view type. Keeps
/// the predicate decoupled from the specific view shape (`LinkView`, `ClassLinkView`,
/// `NeighborLinkView`) each caller holds. The `told_in` field is carried so the marker can resolve
/// the reference, mirroring how content entries carry `told_in` for `MarkerTurn`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkVis {
    pub from: MemoryId,
    pub to: MemoryId,
    pub visibility: Visibility,
    pub told_by: Option<Teller>,
    pub told_in: Option<ConversationRef>,
}

/// A stored edge in its canonical direction, carrying its visibility posture and provenance.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct LinkView {
    pub from: MemoryId,
    pub to: MemoryId,
    pub relation: RelationName,
    /// The teller who asserted the relationship, if one is on record. `None` for links with no
    /// teller behind them (an operator-authored `same_as`) or predating link provenance.
    pub told_by: Option<Teller>,
    /// The conversation reference (turn or room) the link was asserted in, mirroring content
    /// entries' `told_in`.
    pub told_in: Option<ConversationRef>,
    /// The audience posture, governing the read-time `link_visible` predicate.
    pub visibility: Visibility,
}

impl LinkView {
    /// The fields the link visibility predicate reasons over.
    pub fn link_vis(&self) -> LinkVis {
        LinkVis {
            from: self.from,
            to: self.to,
            visibility: self.visibility.clone(),
            told_by: self.told_by.clone(),
            told_in: self.told_in.clone(),
        }
    }
}

/// A stored edge touching a `same_as` class, carrying its `source` so a class-traversing link read
/// keeps the per-edge provenance the agent-facing readers surface (spec §Lua API → link readers).
/// Distinct from [`LinkView`] so the console wire contract over the latter stays untouched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClassLinkView {
    pub from: MemoryId,
    pub to: MemoryId,
    pub relation: RelationName,
    pub source: LinkSource,
    /// The teller who asserted the relationship, if one is on record — `None` for a link with no
    /// teller behind it (an operator-authored `same_as`) or one predating link provenance.
    pub told_by: Option<Teller>,
    /// The conversation reference (turn or room) the link was asserted in, mirroring content
    /// entries' `told_in`.
    pub told_in: Option<ConversationRef>,
    /// The audience posture, governing the read-time `link_visible` predicate.
    pub visibility: Visibility,
}

impl ClassLinkView {
    /// The fields the link visibility predicate reasons over.
    pub fn link_vis(&self) -> LinkVis {
        LinkVis {
            from: self.from,
            to: self.to,
            visibility: self.visibility.clone(),
            told_by: self.told_by.clone(),
            told_in: self.told_in.clone(),
        }
    }
}

/// A neighbor on a memory's out-of-class relation surface — the raw material for the salient-relations
/// line a search hit carries. It names the relation, whether the edge runs *into* this class
/// (`incoming`) or out of it, and the far memory (id plus its resolved name, so a caller renders
/// `relation → name` without a second lookup). The query returns only edges leaving the class — an edge
/// internal to the `same_as` class is identity plumbing, not a relationship — ordered most-recently
/// created first (by the link's insertion `rowid`). Committed state; visibility-filtered through
/// `link_visible` when an audience is present, mirroring the content entry reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NeighborLinkView {
    pub relation: RelationName,
    pub incoming: bool,
    pub other: MemoryId,
    pub other_name: MemoryName,
    /// The `from` endpoint of the stored edge, pre-canonicalization. Needed so the predicate can
    /// reason about which endpoint is the teller and which is the subject.
    pub from: MemoryId,
    /// The `to` endpoint of the stored edge, pre-canonicalization.
    pub to: MemoryId,
    /// The teller who asserted the relationship, if one is on record.
    pub told_by: Option<Teller>,
    /// The conversation reference (turn or room) the link was asserted in, mirroring content
    /// entries' `told_in`.
    pub told_in: Option<ConversationRef>,
    /// The audience posture, governing the read-time `link_visible` predicate.
    pub visibility: Visibility,
}

impl NeighborLinkView {
    /// The fields the link visibility predicate reasons over.
    pub fn link_vis(&self) -> LinkVis {
        LinkVis {
            from: self.from,
            to: self.to,
            visibility: self.visibility.clone(),
            told_by: self.told_by.clone(),
            told_in: self.told_in.clone(),
        }
    }
}

/// A conversation as projected: its id, its locator (the room it addresses), and the context memory
/// that is its room. A conversation whose context memory has been deleted is not projected, so a
/// listing reflects only the live rooms (see [`Graph::conversations`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConversationView {
    pub id: ConversationId,
    pub locator: ConversationLocator,
    pub context_memory: MemoryId,
}

/// A session as projected: its conversation, when it opened, the carryover extent (if it opened via
/// compaction), the captured brief, and its participants (the present set at open, plus anyone who
/// joined mid-session).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionView {
    pub id: SessionId,
    pub conversation: ConversationId,
    pub started_at: Timestamp,
    pub seeded_from_turn: Option<ConversationRef>,
    pub brief: String,
    pub participants: Vec<MemoryId>,
}

/// What reconstructing a live `OpenSession` after a restart needs (see [`Graph::last_open_session`]):
/// the session's id, the brief frozen at its open, when it opened, and the `SessionStarted` seq the
/// live buffer reads from. `seeded` flags a compaction-seam continuation, whose true buffer starts at
/// a carried tail before `start_seq` — so it is not byte-faithfully resumable from the seq alone.
#[derive(Clone, Debug, PartialEq)]
pub struct OpenSessionView {
    pub id: SessionId,
    pub brief: String,
    pub started_at: Timestamp,
    pub start_seq: Seq,
    pub seeded: bool,
}

/// The plan for minting a fresh [`Namespace::Person`] participant stub: the qualified name it
/// receives (`person/<id>@<platform>`). The caller (`resolve_or_mint_participant`) is responsible
/// for checking whether the name already exists as a memory (an agent-authored hearsay stub) and
/// binding the platform identity to it, or creating a fresh memory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParticipantMint {
    pub name: MemoryName,
}
