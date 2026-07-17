//! Idle-reopen carryover (issue #86): a session reopening after an idle gap carries a bounded tail of
//! the previous session's recent messages, so the agent resumes with conversational continuity. The
//! tail is reconstructed from the event log at reopen (not cached across the close), so every seam
//! carries one — a budget compaction and an idle reopen alike. These tests drive the close through the
//! real `ensure_session` hook (`route_message` after the clock has advanced past the idle gap) and
//! assert the reopened session seeds from the prior session's tail; the restart-survival counterpart
//! (a fresh instance over the same log) lives in `tests/server/routing.rs`.

use super::*;
use crate::{
    ConversationLocator, PersonId, SeedSelf, TEST_PLATFORM,
    clock::ManualClock,
    event::EventPayload,
    ids::{ConversationId, Seq},
    model::{Completion, ScriptedModel},
    time::Timestamp,
};

/// Boot and birth a fresh in-memory agent against `clock`, so the genesis templates are registered and
/// turns can run.
fn born_server(clock: ManualClock) -> Instance {
    let server = Instance::in_memory(Box::new(clock)).unwrap();
    server
        .control()
        .create_agent(&SeedSelf {
            agent_name: "Kestrel".to_owned(),
            persona: "A discreet companion with a long memory.".to_owned(),
            seed_entries: vec![],
        })
        .unwrap();
    server
}

/// The conversation id for a locator that has been seen this run.
fn conversation_of(server: &Instance, locator: &ConversationLocator) -> ConversationId {
    server
        .engine
        .graph
        .lock()
        .conversation_for_locator(locator)
        .unwrap()
        .expect("the room has been seen")
}

/// Every `SessionStarted`'s `seeded_from_turn`, in commit order — `None` for a cold open, `Some` for a
/// session seeded from a carried tail.
fn session_seeds(server: &Instance) -> Vec<Option<crate::event::ConversationRef>> {
    server
        .control()
        .events()
        .unwrap()
        .iter()
        .filter_map(|event| match &event.payload {
            EventPayload::SessionStarted {
                seeded_from_turn, ..
            } => Some(seeded_from_turn.clone()),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn a_session_reopening_after_an_idle_gap_carries_the_prior_tail() {
    let clock = ManualClock::new(Timestamp::from_millis(1_000));
    let server = born_server(clock.clone());
    let room = ConversationLocator::new(TEST_PLATFORM, "room-a");
    let dave = PersonId::new(TEST_PLATFORM, "dave");
    // Call 0: the first session's reply. Call 1: the reopened session's reply. The session is light
    // (two turns, below `flush_min_turns`), so the idle close records `SessionEnded` without a flush
    // turn — the carryover is staged regardless, from the light session's own turns.
    let model = ScriptedModel::new([
        Completion::Reply("let's start with the migration".to_owned()),
        Completion::Reply("still on Friday".to_owned()),
    ]);

    // The first session opens and records dave's turn plus the agent's reply.
    server
        .platform()
        .route_message(
            &model,
            &room,
            &dave,
            "when does the migration ship?",
            std::slice::from_ref(&dave),
        )
        .await
        .unwrap();
    let conversation = conversation_of(&server, &room);
    let first_session = server
        .sessions
        .get(conversation)
        .expect("the first session is live")
        .id;

    // Go quiet past the idle gap (default 30 minutes), then message again: `ensure_session` finds the
    // live session stale, closes it (staging the carryover), and opens a fresh one seeded from the tail.
    clock.advance_millis(31 * 60 * 1_000);
    server
        .platform()
        .route_message(
            &model,
            &room,
            &dave,
            "remind me — when does it ship?",
            std::slice::from_ref(&dave),
        )
        .await
        .unwrap();

    let reopened = server
        .sessions
        .get(conversation)
        .expect("the reopened session is live");
    assert_ne!(
        reopened.id, first_session,
        "the idle gap must open a fresh session, not reuse the stale one"
    );

    // The reopened session's `SessionStarted` records the carried tail's extent, and the first
    // session's was a cold open (no prior tail).
    let seeds = session_seeds(&server);
    assert_eq!(seeds.len(), 2, "two sessions opened");
    assert!(
        seeds[0].is_none(),
        "the first session is a cold open — no prior tail to carry"
    );
    let seeded = seeds[1]
        .as_ref()
        .expect("the reopened session seeds from the prior session's tail");
    assert_eq!(seeded.conversation, conversation);
    assert!(
        seeded.turn.is_some(),
        "the carried tail names the oldest carried turn"
    );

    // The reopened session's buffer read starts below its own `SessionStarted` — it reads the carried
    // tail plus its own turns, not only its own (a fresh session would have `start_seq ==
    // session_start_seq`).
    assert!(
        reopened.start_seq < reopened.session_start_seq,
        "the reopened session reads a carried tail before its own turns ({:?} < {:?})",
        reopened.start_seq,
        reopened.session_start_seq,
    );

    // The carried tail's extent is a real turn from the first session — the oldest turn the budget
    // admits, which for a light session is its first turn.
    let seeded_seq = event_seq_of_turn(&server, seeded.turn.unwrap());
    assert_eq!(
        reopened.start_seq, seeded_seq,
        "the buffer read starts at the carried tail's oldest turn"
    );

    // End to end: the previous session's messages actually reach the model. The reopened session's turn
    // is the model's second `generate` call (session 1's reply was the first), and its prompt suffix —
    // the replayed buffer — must carry the first session's own turns, both dave's message and the
    // agent's reply, not just this session's fresh one.
    let calls = model.recorded_messages();
    assert_eq!(calls.len(), 2, "one model call per session turn");
    let reopened_prompt: String = calls[1]
        .iter()
        .map(|message| message.content.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        reopened_prompt.contains("when does the migration ship?"),
        "the reopened prompt must carry the prior session's participant message; got:\n{reopened_prompt}"
    );
    assert!(
        reopened_prompt.contains("let's start with the migration"),
        "the reopened prompt must carry the prior session's agent reply; got:\n{reopened_prompt}"
    );
}

/// The seq of the `ConversationTurn` bearing `turn_id`.
fn event_seq_of_turn(server: &Instance, turn_id: crate::ids::TurnId) -> Seq {
    server
        .control()
        .events()
        .unwrap()
        .iter()
        .find_map(|event| match &event.payload {
            EventPayload::ConversationTurn { turn_id: id, .. } if *id == turn_id => Some(event.seq),
            _ => None,
        })
        .expect("the carried turn is in the log")
}
