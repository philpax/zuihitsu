//! The session machinery shared by both facets: opening/continuing a session and running one turn,
//! plus the supporting runtime types (the routed-turn bundle, the compaction carryover, and the live
//! open-session backing a conversation). On [`super::Instance`] (not a facet) so the platform
//! `route_message` and the operator `imprint` both reach it.

mod join;
mod lifecycle;
mod turn;

use std::sync::atomic::{AtomicI64, Ordering};

use crate::{
    agent::{lua::Session, turn::InboundMessage},
    event::PromptTemplateName,
    ids::{ConversationId, MemoryId, Seq, SessionId, TurnId},
    memory::memory_block::Authority,
    time::Timestamp,
};

/// The raw-transcript carryover a compaction stages for the next session (spec §Compaction →
/// raw-transcript carryover). The oldest carried turn is both the `seeded_from_turn` boundary
/// recorded on the new `SessionStarted` and the `from_seq` the new session's buffer is read from, so
/// the carried tail plus the new turns reconstruct the post-cut buffer.
pub(crate) struct Carryover {
    pub seeded_from_turn: TurnId,
    pub from_seq: Seq,
    /// The memories the ending session touched (read or wrote), re-surfaced in the new session's
    /// brief as active threads — the touch-derived working set (spec §Compaction → working-set
    /// carryover).
    pub working_set: Vec<MemoryId>,
}

/// The live session backing a conversation (runtime state, see [`super::Instance::sessions`]). Held
/// behind an `Arc` in the `sessions` map, so a running turn keeps its session alive without the map
/// guard; only `last_activity` is mutated after open, so it is an atomic the reuse path bumps through
/// `&self`.
pub(crate) struct OpenSession {
    pub id: SessionId,
    pub vm: Session,
    pub brief: String,
    /// The memory ids the frozen brief reads over — the present set, the working set, the current
    /// room's context, and self. Threaded into each turn so the ambient recall pass can exclude what
    /// the brief already surfaces (see [`super::super::agent::Turn::brief_memories`]).
    pub brief_memories: Vec<MemoryId>,
    /// When the session opened — the time frozen into the system prompt's "the session begins on …",
    /// so every turn in the session sends an identical system prefix (the live wall clock rides in the
    /// per-message stamps instead). Holding it stable is what lets the serving layer reuse the prefix
    /// cache across the session's turns.
    pub started_at: Timestamp,
    /// The last-activity wall-clock in epoch millis, the idle-gap is measured from. Atomic so the
    /// idle-reuse path can bump it through the shared `&OpenSession` without a map-wide write lock.
    pub last_activity: AtomicI64,
    /// The log seq the live buffer is read from: the `SessionStarted` seq for a fresh or idle-opened
    /// session, or a carried tail's seq across a compaction seam (so the carryover plus this
    /// session's turns reconstruct the buffer — see [`buffer_turns`]).
    pub start_seq: Seq,
    /// This session's own `SessionStarted` seq — where its own turns begin, at or after `start_seq`.
    /// It splits the buffer read at turn time (and at the flush): the carried tail below it is
    /// re-trimmed to the carryover char budget, while this session's own turns ride whole, so the
    /// buffer stays bounded across compaction seams (see [`bounded_buffer_turns`]). Equal to
    /// `start_seq` for a fresh or idle-opened session (an empty tail).
    pub session_start_seq: Seq,
}

impl OpenSession {
    /// The last-activity time in epoch millis.
    pub fn last_activity_millis(&self) -> i64 {
        self.last_activity.load(Ordering::Relaxed)
    }

    /// Record `now` as the last activity (the idle-reuse bump).
    pub fn touch(&self, now: Timestamp) {
        self.last_activity.store(now.as_millis(), Ordering::Relaxed);
    }
}

/// One routed turn — the inbound message and its routing context, bundled so
/// [`super::Instance::run_session_turn`] takes the routed turn as a whole. Shared by the platform
/// `route_messages` and the operator `imprint` paths.
pub(crate) struct RoutedTurn<'a> {
    pub conversation: ConversationId,
    pub present_set: &'a [MemoryId],
    /// The inbound participant messages for this turn. Each carries its own speaker and text;
    /// the agent response cycle runs once for the whole batch.
    pub inbound: &'a [InboundMessage],
    /// The participant turn ids already recorded by the caller (one per inbound message). Passed
    /// through so `run_turn` can return them in the `TurnReport` without recording the turns itself.
    pub participant_turn_ids: &'a [TurnId],
    pub template: PromptTemplateName,
    pub authority: Authority,
}
