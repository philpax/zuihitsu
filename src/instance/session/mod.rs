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
        agent::ToolStep,
        attachment::{Attachment, AttachmentKind},
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
    fn the_trim_carries_the_exchanges_whose_growth_fits() {
        // Three calls bracket two spans of 1,000 tokens. The cut lands just after a call, so the
        // messages it answered ride with its answer.
        let buffer = vec![
            measured(1, "a", 1_000, 0),
            turn(2, "the message the next call answered"),
            measured(3, "b", 2_000, 0),
            turn(4, "and the next"),
            measured(5, "c", 3_000, 0),
        ];
        // One span fits: everything after the middle call, which is the turn it answered and itself.
        assert_eq!(carryover_start(&buffer, 1_000), 3);
        // Both spans fit, and the turns before the oldest call ride free — nothing prices them.
        assert_eq!(carryover_start(&buffer, 2_000), 0);
        // Nothing fits: the newest turn is carried regardless.
        assert_eq!(carryover_start(&buffer, 999), 4);
    }

    #[test]
    fn the_newest_exchange_is_carried_without_being_charged() {
        // Nothing has bracketed the newest call's own generation, so it is not priced — and it rides
        // whatever else is carried rather than being guessed at.
        let buffer = vec![
            measured(1, "a", 1_000, 0),
            measured(2, "b", 2_000, 0),
            turn(3, &"an unreported turn after the newest call".repeat(50)),
        ];
        assert_eq!(carryover_start(&buffer, 1_000), 0);
    }

    #[test]
    fn a_buffer_with_no_recorded_call_carries_its_newest_turn() {
        // Every turn deferred, so no call ever completed. Nothing about this buffer is known, and the
        // newest turn is carried regardless.
        let buffer = vec![turn(1, "a"), turn(2, "b"), turn(3, "c")];
        assert_eq!(carryover_start(&buffer, 10_000), 2);
    }

    #[test]
    fn an_attachment_is_priced_by_the_backend_that_charged_for_it() {
        // The file is inside the reported growth, so it needs no guessing at: the span between the
        // two calls costs what the backend said, whatever the sharing turn's text length.
        let mut sharing = turn(2, "have a look");
        sharing.attachments = vec![Attachment {
            name: "notes.txt".to_owned(),
            mime: "text/plain".into(),
            blob: crate::ids::BlobHash::of(b"notes"),
            byte_len: 8_000,
            kind: AttachmentKind::Text,
        }];
        let buffer = vec![
            measured(1, "a", 1_000, 0),
            sharing,
            measured(3, "", 3_000, 0),
        ];
        // The sharing turn is inside the 2,000-token growth, so a budget under it keeps only the
        // newest turn, and one over it reaches back past the file.
        assert_eq!(carryover_start(&buffer, 1_999), 2);
        assert_eq!(carryover_start(&buffer, 2_000), 0);
    }

    #[test]
    fn a_prompt_that_shrank_across_the_span_does_not_evict_the_tail() {
        // A tail spanning a seam is priced against a prefix re-frozen in between, so the growth can
        // read negative. It must never wrap into an enormous cost that evicts everything.
        let buffer = vec![measured(1, "aaaa", 5_000, 0), measured(2, "bbbb", 1_000, 0)];
        assert_eq!(carryover_start(&buffer, 10), 0);
    }

    #[test]
    fn carryover_tail_names_the_oldest_carried_turn() {
        let buffer = vec![
            measured(1, "a", 1_000, 0),
            turn(2, "answered by the next call"),
            measured(3, "b", 2_000, 0),
        ];
        // The whole buffer fits, and the turns before the oldest call ride free.
        let carry = carryover_tail(&buffer, 1_000).expect("a non-empty buffer carries a tail");
        assert_eq!(carry.from_seq, Seq(1));
        assert_eq!(carry.seeded_from_turn, buffer[0].turn_id);

        // Under the older span's cost, the tail starts after the newest call — which, with nothing
        // recorded since, is the newest turn itself.
        let carry = carryover_tail(&buffer, 999).expect("a non-empty buffer carries a tail");
        assert_eq!(carry.from_seq, Seq(3));
        assert_eq!(carry.seeded_from_turn, buffer[2].turn_id);

        assert!(carryover_tail(&[], 100).is_none());
    }
}
