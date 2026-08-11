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

/// The raw-transcript tail of `buffer`: the most recent turns that fit `token_budget`, filled backward
/// from the end (spec §Compaction → raw-transcript carryover). The newest turn is always carried so the
/// immediate conversational thread survives the seam, then older turns are added while they fit.
/// Returns the oldest carried turn as the tail extent, or `None` for an empty buffer. Called at reopen
/// against the previous session's own turns to derive the seed (see
/// [`crate::instance::Instance::ensure_session`]).
pub(crate) fn carryover_tail(buffer: &[TurnView], token_budget: i64) -> Option<TailSeed> {
    let start = carryover_start(buffer, token_budget);
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
    /// re-trimmed to the carryover token budget, while this session's own turns ride whole, so the
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
        self.last_activity
            .store(now.as_millisecond(), Ordering::Relaxed);
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
    use crate::{
        agent::{ToolStep, turn_token_costs},
        event::TurnRole,
    };

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
            prompt_tokens: None,
            attachments: Vec::new(),
        }
    }

    #[test]
    fn carryover_tail_admits_the_newest_turns_that_fit_the_budget() {
        // Unmeasured turns fall back to `chars / 4`: texts of 16, 16, and 8 chars cost 4, 4, and 2.
        let buffer = vec![
            turn(1, &"a".repeat(16)),
            turn(2, &"b".repeat(16)),
            turn(3, &"c".repeat(8)),
        ];
        // Budget 6 admits the newest (2) plus the next (4) = 6, but not the third — extent is seq 2.
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
        let buffer = vec![
            turn(1, &"a".repeat(16)),
            turn(2, &"b".repeat(16)),
            turn(3, &"c".repeat(8)),
        ];
        // Budget 6 admits the newest (2) plus the next (4) = 6, not the third — the tail starts at 1.
        assert_eq!(carryover_start(&buffer, 6), 1);
        // A budget below the newest turn still keeps it (index 2), never an empty tail.
        assert_eq!(carryover_start(&buffer, 0), 2);
        // A budget the whole buffer fits keeps everything (index 0).
        assert_eq!(carryover_start(&buffer, 1_000), 0);
        // An empty slice keeps nothing — the past-the-end index.
        assert_eq!(carryover_start(&[], 100), 0);
    }

    /// An agent turn whose first model call reported `prompt_tokens`, carrying `steps` a character
    /// count cannot see.
    fn measured(seq: u64, text: &str, prompt_tokens: u32, step_chars: usize) -> TurnView {
        TurnView {
            role: TurnRole::Agent,
            steps: vec![ToolStep {
                script: "x".repeat(step_chars),
                result: String::new(),
            }],
            prompt_tokens: Some(prompt_tokens),
            ..turn(seq, text)
        }
    }

    #[test]
    fn the_trim_prices_a_span_by_the_reported_prompt_sizes_not_its_characters() {
        // Two spans of one turn each. The texts are the same length, so a character count would price
        // them identically — but the backend reported the first span costing 100 tokens and the
        // second 1000, which is what the tool steps inside them actually cost.
        let buffer = vec![
            measured(1, "aaaa", 1_000, 0),
            measured(2, "bbbb", 1_100, 8_000),
            measured(3, "", 2_100, 0),
        ];
        // The newest turn closes the second span and is itself unpriced (nothing followed it), but
        // it holds no text, so it costs nothing and the two spans stand alone.
        assert_eq!(turn_token_costs(&buffer), vec![100, 1_000, 0]);
        // A budget below 1000 cannot afford the second span, so only the newest turn is kept.
        assert_eq!(carryover_start(&buffer, 999), 2);
        // 1100 affords both spans, reaching back to the oldest turn.
        assert_eq!(carryover_start(&buffer, 1_100), 0);
    }

    #[test]
    fn a_turn_the_backend_never_priced_falls_back_to_the_character_estimate() {
        // No turn carries a prompt size, so every turn is estimated at `chars / 4`.
        let buffer = vec![turn(1, &"a".repeat(40)), turn(2, &"b".repeat(40))];
        assert_eq!(turn_token_costs(&buffer), vec![10, 10]);
    }

    #[test]
    fn a_prompt_that_shrank_across_the_span_is_clamped_to_zero() {
        // A tail spanning a seam is priced against a prefix re-frozen in between, so the difference
        // can read negative. It must never wrap into an enormous cost that evicts the whole tail.
        let buffer = vec![measured(1, "aaaa", 5_000, 0), measured(2, "bbbb", 1_000, 0)];
        assert_eq!(turn_token_costs(&buffer)[0], 0);
    }

    #[test]
    fn a_span_of_several_turns_splits_its_measured_cost_between_them() {
        // One measured span covering three turns: the 900 tokens are shared by character weight
        // (10/20/60 chars → 1/2/6 of the total), and the shares sum to the measurement exactly.
        let buffer = vec![
            measured(1, &"a".repeat(10), 1_000, 0),
            turn(2, &"b".repeat(20)),
            turn(3, &"c".repeat(60)),
            measured(4, "d", 1_900, 0),
        ];
        let costs = turn_token_costs(&buffer);
        assert_eq!(costs[..3].iter().sum::<usize>(), 900);
        assert_eq!(costs[..3], [100, 200, 600]);
    }
}
