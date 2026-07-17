//! The session machinery shared by both facets: opening/continuing a session and running one turn,
//! plus the supporting runtime types (the routed-turn bundle, the compaction carryover, and the live
//! open-session backing a conversation). On [`crate::instance::Instance`] (not a facet) so the platform
//! `route_message` and the operator `imprint` both reach it.

mod join;
mod lifecycle;
mod turn;

use std::sync::atomic::{AtomicI64, Ordering};

use crate::{
    agent::{TurnView, carryover_start, lua::Session, turn::InboundMessage},
    event::PromptTemplateName,
    ids::{ConversationId, MemoryId, Seq, SessionId, TurnId},
    memory::memory_block::Authority,
    time::Timestamp,
};

/// The extent of the raw-transcript tail a reopen seeds the next session's buffer from (spec
/// §Compaction → raw-transcript carryover). The oldest carried turn is both the `seeded_from_turn`
/// boundary recorded on the new `SessionStarted` and the `from_seq` the new buffer is read from, so the
/// carried tail plus the new turns reconstruct the post-seam buffer. Reconstructed from the log at open
/// time — the previous session's own turns are all in the event log, so nothing is cached across the
/// close (issue #86).
pub(crate) struct TailSeed {
    pub seeded_from_turn: TurnId,
    pub from_seq: Seq,
}

/// The raw-transcript tail of `buffer`: the most recent turns that fit `char_budget`, filled backward
/// from the end (spec §Compaction → raw-transcript carryover). The newest turn is always carried so the
/// immediate conversational thread survives the seam, then older turns are added while they fit.
/// Returns the oldest carried turn as the tail extent, or `None` for an empty buffer. Called at reopen
/// against the previous session's own turns to derive the seed (see
/// [`crate::instance::Instance::ensure_session`]).
pub(crate) fn carryover_tail(buffer: &[TurnView], char_budget: i64) -> Option<TailSeed> {
    let start = carryover_start(buffer, char_budget);
    buffer.get(start).map(|turn| TailSeed {
        seeded_from_turn: turn.turn_id,
        from_seq: turn.seq,
    })
}

/// The live session backing a conversation (runtime state, see [`crate::instance::Instance::sessions`]). Held
/// behind an `Arc` in the `sessions` map, so a running turn keeps its session alive without the map
/// guard; only `last_activity` is mutated after open, so it is an atomic the reuse path bumps through
/// `&self`.
pub(crate) struct OpenSession {
    pub id: SessionId,
    pub vm: Session,
    pub brief: String,
    /// The memory ids the frozen brief reads over — the present set, the working set, the current
    /// room's context, and self. Threaded into each turn so the ambient recall pass can exclude what
    /// the brief already surfaces (see [`crate::agent::Turn::brief_memories`]).
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
/// [`crate::instance::Instance::run_session_turn`] takes the routed turn as a whole. Shared by the platform
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::TurnRole;

    fn turn(seq: u64, text: &str) -> TurnView {
        TurnView {
            seq: Seq(seq),
            turn_id: TurnId::generate(),
            role: TurnRole::Participant,
            text: text.to_owned(),
            participant: None,
            recorded_at: Timestamp::from_millis(0),
            steps: Vec::new(),
            produced_by: None,
        }
    }

    #[test]
    fn carryover_tail_admits_the_newest_turns_that_fit_the_budget() {
        // Texts of 4, 4, and 2 chars, newest last.
        let buffer = vec![turn(1, "aaaa"), turn(2, "bbbb"), turn(3, "cc")];
        // Budget 6 admits "cc" (2) + "bbbb" (4) = 6, but not the next "aaaa" — extent is seq 2.
        let carry = carryover_tail(&buffer, 6).expect("a non-empty buffer carries a tail");
        assert_eq!(carry.from_seq, Seq(2));
        assert_eq!(carry.seeded_from_turn, buffer[1].turn_id);
    }

    #[test]
    fn carryover_tail_always_keeps_the_newest_turn_even_over_budget() {
        let buffer = vec![
            turn(1, "short"),
            turn(2, "a long final turn that alone exceeds the budget"),
        ];
        // The immediate thread survives the seam: the newest turn is carried regardless.
        let carry = carryover_tail(&buffer, 1).expect("the newest turn is always carried");
        assert_eq!(carry.from_seq, Seq(2));
        assert_eq!(carry.seeded_from_turn, buffer[1].turn_id);
    }

    #[test]
    fn carryover_tail_of_an_empty_buffer_is_none() {
        assert!(carryover_tail(&[], 100).is_none());
    }

    #[test]
    fn carryover_start_indexes_the_oldest_turn_that_fits() {
        let buffer = vec![turn(1, "aaaa"), turn(2, "bbbb"), turn(3, "cc")];
        // Budget 6 admits "cc" (2) + "bbbb" (4) = 6, not "aaaa" — the kept tail starts at index 1.
        assert_eq!(carryover_start(&buffer, 6), 1);
        // A budget below the newest turn still keeps it (index 2), never an empty tail.
        assert_eq!(carryover_start(&buffer, 0), 2);
        // A budget the whole buffer fits keeps everything (index 0).
        assert_eq!(carryover_start(&buffer, 1_000), 0);
        // An empty slice keeps nothing — the past-the-end index.
        assert_eq!(carryover_start(&[], 100), 0);
    }
}
