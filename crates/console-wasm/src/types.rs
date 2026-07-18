use serde::Serialize;
use zuihitsu_core::{
    event::MergeProposalSource,
    graph::{EntryView, LinkView, MemoryView},
    ids::{ConversationId, EntryId, MemoryId, MemoryName, SessionId},
    time::Timestamp,
};

/// Everything the State view shows when a memory is opened: the memory itself, its live content
/// entries, its full history (including superseded entries), its links, and its `same_as` class.
/// Composed from several core reads so the frontend opens a memory in one call.
#[derive(Serialize)]
pub struct MemoryDetail {
    pub memory: MemoryView,
    pub entries: Vec<EntryView>,
    pub history: Vec<EntryView>,
    pub links: Vec<LinkView>,
    pub class: Vec<MemoryView>,
    /// The entry ids currently under an unresolved belief arbitration, so the view can mark a contested
    /// fact as disputed (the same signal the agent sees on a read).
    pub disputed: Vec<EntryId>,
}

/// One cross-platform merge proposal as the console surfaces it (spec §Cross-platform identity): the
/// two stubs by handle *and* id (so the view can name them and deep-link into State), who raised it,
/// the proposer's stated grounds if any, and where the proposal now stands. Unlike the operator
/// backstop — which drops a settled proposal — the console keeps every proposal so it can show the
/// whole record: which pairs the agent flagged and which the operator has since confirmed.
#[derive(Serialize)]
pub struct MergeProposalView {
    pub from: MemoryName,
    pub to: MemoryName,
    pub from_id: MemoryId,
    pub to_id: MemoryId,
    pub source: MergeProposalSource,
    /// The proposer's stated grounds for the match — the coincidence the agent reasoned from. `None` for
    /// an orchestration handle match or a `same_as`-via-link, which carry no rationale.
    pub rationale: Option<String>,
    pub status: MergeStatus,
    /// Whether each stub is currently its `same_as` class's primary — the id class-level reads resolve
    /// through — so the view marks the canonical stub. Once merged exactly one of a pair is primary,
    /// unless a third, older member holds the class, in which case neither is.
    pub from_primary: bool,
    pub to_primary: bool,
    /// Whether the operator has pinned each stub as its class's primary (`ClassPrimaryDesignated`), as
    /// opposed to it winning by the earliest-ULID default. The view offers a pinned stub a release and
    /// an unpinned, non-primary stub a designation.
    pub from_designated: bool,
    pub to_designated: bool,
}

/// Where a merge proposal stands at the current fold horizon: still awaiting the operator's decision, or
/// merged (the operator confirmed it and the two stubs now share a `same_as` class).
#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MergeStatus {
    Pending,
    Merged,
}

/// One item on the agent's agenda: when it occurs, the memory it lives in, the text, and whether it
/// is a recurring instance. One-offs come from `occurrences_in_window`; recurring instances from
/// `recurring_instances_in_window`, which expands each rule through the agent's own `next_occurrence`
/// so the projection cannot drift from the agent's scheduling.
#[derive(Serialize)]
pub struct AgendaItem {
    pub when: Timestamp,
    /// The occurrence is a whole day or fuzzier span, not a precise instant, so the calendar renders
    /// it without a clock time (a `Day` sorts at noon — not a stated time). See `TemporalRef::is_all_day`.
    pub all_day: bool,
    pub memory: String,
    pub text: String,
    pub recurring: bool,
}

/// A durable conversation (room) with its sessions, the backbone of the Conversation view. The
/// turns themselves render off the event stream; this supplies the structure and the names the raw
/// log only carries as ids — the room's [`Namespace::Context`] name and each session's participant
/// handles.
#[derive(Serialize)]
pub struct ConversationDetail {
    pub id: ConversationId,
    pub platform: String,
    pub scope_path: String,
    pub context_name: Option<String>,
    pub sessions: Vec<SessionSummary>,
}

/// One activity window within a conversation: when it opened, the brief frozen at its start, and the
/// participants present, resolved to their memory handles.
#[derive(Serialize)]
pub struct SessionSummary {
    pub id: SessionId,
    pub started_at: Timestamp,
    pub brief: String,
    pub participants: Vec<String>,
}
