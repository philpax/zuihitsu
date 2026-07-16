//! The operator-authority facet: agent creation and read-only inspection. A platform client can
//! never obtain one of these, which is what keeps the operator surface off the platform boundary
//! (spec §Clients and the server boundary).

mod actions;
mod inspect;

use serde::{Deserialize, Serialize};

use crate::{
    event::{MergeProposalSource, ModelPhase, RequestRecord},
    ids::{ConversationId, EntryId, MemoryId, MemoryName, Seq, TurnId},
    model::{Completion, Usage},
    time::Timestamp,
};

/// One entry to write to a conversation's context memory via [`Platform::write_context`]. A typed
/// alternative to interpolating untrusted strings into a Lua script — the connector posts structured
/// data, and the server handles the memory write directly.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ContextEntry {
    /// The entry's text content.
    pub text: String,
}

/// Operator-authority operations: agent creation and read-only inspection. A platform client can
/// never obtain one of these.
pub struct Control<'a> {
    pub(super) server: &'a crate::instance::Instance,
}

/// One recorded belief arbitration: the memory it concerns and the reconciling statement the agent
/// wrote (spec §Write path). The operator/console view of "why does it believe X".
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Arbitration {
    pub memory: MemoryName,
    pub statement: String,
}

/// One cross-platform merge proposal still awaiting the operator (spec §Cross-platform identity →
/// adjudicated merge): the two stubs, who raised it, and whether the adjudicator has already weighed and
/// refused it. A proposal the adjudicator (or an operator) has *accepted* — the two stubs now share a
/// `same_as` class — drops off; every other proposal stays, so the "left for the operator" path is
/// visible here rather than silently dropped. The operator's backstop for merges the evidence did not
/// (yet) justify, including the orchestration-raised ones from a bare handle match.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MergeProposal {
    pub from: MemoryName,
    pub to: MemoryName,
    pub source: MergeProposalSource,
    /// `true` once the adjudicator has weighed the pair and refused the merge; `false` while it is still
    /// unweighed (or the adjudicator could not reach a verdict). Either way the operator can act on it.
    pub refused: bool,
}

/// One recorded model interaction — the console's view of a single model call (spec
/// §Observability). The `seq` and `recorded_at` of the `ModelCalled` event place the call on the
/// timeline; the rest mirrors the event. The `request` is delta-encoded (`Base`/`Continuation`); the
/// console reconstructs a full prompt by walking a `(turn_id, phase)` group, and `request_digest`
/// checks the reconstruction against the call actually sent.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelCall {
    pub seq: Seq,
    pub recorded_at: Timestamp,
    pub conversation: ConversationId,
    pub turn_id: TurnId,
    pub phase: ModelPhase,
    pub request_digest: String,
    pub request: Option<RequestRecord>,
    pub completion: Completion,
    pub reasoning: Option<String>,
    pub finish_reason: Option<String>,
    pub usage: Usage,
    pub duration_ms: u64,
}

/// The result of one operator Lua console run (spec §Observability → the operator Lua console): the
/// rendered value of the block's final expression, or the error/abort that ended it. Exactly one is
/// `Some`. The run is a no-commit sandbox — nothing it writes persists — so it leaves no trace on the
/// log.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LuaConsoleOutcome {
    pub result: Option<String>,
    pub error: Option<String>,
}

/// The outcome of an operator unmerge (`Control::unmerge`): the retraction of an operator-asserted
/// `same_as` merge, or the reason the request named nothing retractable. The console-only undo of a
/// wrong cross-platform merge (spec §Cross-platform identity → operator-asserted merge), the mirror of
/// [`Control::resolve_merge`]'s accept.
pub enum UnmergeOutcome {
    /// The `same_as` edge was removed and the graph re-materialized, so the two identities split back
    /// into their own visibility classes on the next read.
    Removed,
    /// One of the two ids does not resolve to a live memory, so there is nothing to unmerge.
    UnknownMemory(MemoryId),
    /// The two memories share no direct `same_as` edge, so there is no merge to lift between exactly
    /// this pair — only a direct edge is retractable, never a class two stubs joined transitively.
    NotMerged,
}

/// The outcome of an operator primary designation (`Control::designate_primary`): the stub was pinned
/// (or released) as its `same_as` class's primary, or the request named no live memory. The console's
/// lever for choosing which stub a merged class resolves through, over the earliest-ULID default.
pub enum DesignateOutcome {
    /// The designation was recorded and the graph re-materialized, so the class resolves through the
    /// pinned stub (or falls back to earliest-ULID on a release) from the next read.
    Designated,
    /// The id does not resolve to a live memory, so there is nothing to designate.
    UnknownMemory(MemoryId),
}

/// The outcome of an operator `self` edit ([`Control::edit_self`]): the entry the write appended, or the
/// reason the request named nothing writable. The console-direct counterpart to the imprint interview —
/// the operator editing the agent's own profile under operator authority (spec §Imprint interview → the
/// operator owns `self`), the same authority that lets the console edit the scaffold and settings.
#[derive(Debug)]
pub enum SelfEditOutcome {
    /// The edit applied: the new charter entry (the replacement, when the edit superseded a prior
    /// entry, which then drops from every live surface while remaining in history). The graph was
    /// re-materialized, so `self` reads the new state on the next read.
    Applied(EntryId),
    /// The agent has no `self` yet — genesis has not run — so there is nothing to edit.
    NotBorn,
    /// The edit carried no text (empty or whitespace only); a `self` entry must have content.
    EmptyText,
    /// `supersedes` named an entry that is not a live entry of `self`, so there is nothing to replace.
    UnknownEntry(EntryId),
    /// The text exceeds the per-entry character limit; the operator shortens it and retries.
    TooLong { length: usize, limit: usize },
}

/// The outcome of an operator entry retraction ([`Control::retract_entry`]): the entry was
/// tombstoned, or the reason the request named nothing retractable. The console's lever for
/// withdrawing a fact outright — the entry drops from every live surface while remaining in
/// history with its reason.
#[derive(Debug)]
pub enum RetractOutcome {
    /// The retraction applied: the entry is tombstoned, dropping from live surfaces while remaining
    /// in history with its reason. The graph was re-materialized, so the next read reflects it.
    Retracted,
    /// The memory name does not resolve to a live memory, so there is nothing to retract from.
    UnknownMemory,
    /// The entry id is not a live entry of the named memory (or its `same_as` class).
    UnknownEntry(EntryId),
    /// The reason was empty or whitespace only; a retraction must be auditable.
    EmptyReason,
}

/// Order a merge pair so `(a, b)` and `(b, a)` coalesce — `same_as` is symmetric, so a proposal and its
/// adjudication key on the same canonical pair regardless of which stub each named first.
pub(super) fn canonical_pair(from: MemoryId, to: MemoryId) -> (MemoryId, MemoryId) {
    if from <= to { (from, to) } else { (to, from) }
}
