use serde::Serialize;
use zuihitsu_core::{graph::MemoryView, ids::MemoryId, time::Timestamp};

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
