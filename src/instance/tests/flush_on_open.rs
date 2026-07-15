//! The session-open checkpoint flush (issue #60): when a fresh session opens for a conversation, the
//! *other* live conversations are checkpoint-flushed first, so their working state reaches memory
//! before the opening session's brief composes and its first turn dispatches — the backend processes
//! the flush ahead of the new turn rather than thrashing between them. These tests drive the trigger
//! through `route_message` (the real `ensure_session` hook) and, for the gate-isolation cases, call
//! `checkpoint_live_sessions` directly with each [`CheckpointTrigger`].

use super::*;
use crate::{
    CheckpointTrigger, ConversationLocator, PersonId, SeedSelf, TEST_PLATFORM,
    clock::ManualClock,
    event::{EventPayload, PromptTemplateName, TurnRole},
    ids::{ConversationId, Seq},
    model::{Completion, ScriptedModel},
    settings::CheckpointSettings,
    time::Timestamp,
};

/// A substantive room message that clears a low substance threshold on its own.
const SUBSTANTIVE: &str = "We decided: the migration ships on Friday, Erin owns the comms, and the fallback window is \
     Monday morning — please note all three.";

/// Boot and birth a fresh in-memory agent against `clock`, so the genesis templates (Scaffold and
/// Flush) are registered and turns can run.
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

/// Point the checkpoint gates at the test's scale, choosing whether a session open sweeps the other
/// rooms — the substance threshold and cooldown, leaving the rest of the settings as seeded.
fn tune_checkpoint(
    server: &Instance,
    min_delta_chars: i64,
    cooldown_seconds: i64,
    flush_on_open: bool,
) {
    let mut settings = server.control().settings().unwrap();
    settings.checkpoint = CheckpointSettings {
        enabled: true,
        min_delta_chars,
        cooldown_seconds,
        flush_on_open,
    };
    server.control().set_settings(settings).unwrap();
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

/// Every Flush-provenance `ConversationTurn` seq in the log — a checkpoint (or end-)flush turn's
/// signature.
fn flush_turn_seqs(server: &Instance) -> Vec<Seq> {
    server
        .control()
        .events()
        .unwrap()
        .iter()
        .filter_map(|event| match &event.payload {
            EventPayload::ConversationTurn {
                produced_by: Some(produced),
                ..
            } if produced.template_name == PromptTemplateName::Flush => Some(event.seq),
            _ => None,
        })
        .collect()
}

/// The first seq of a participant `ConversationTurn` whose text is exactly `text`.
fn participant_turn_seq(server: &Instance, text: &str) -> Seq {
    server
        .control()
        .events()
        .unwrap()
        .iter()
        .find_map(|event| match &event.payload {
            EventPayload::ConversationTurn {
                role: TurnRole::Participant,
                text: turn_text,
                ..
            } if turn_text == text => Some(event.seq),
            _ => None,
        })
        .expect("the participant turn is recorded")
}

#[tokio::test]
async fn a_new_conversation_flushes_the_prior_one_before_its_first_turn() {
    let server = born_server(ManualClock::new(Timestamp::from_millis(1_000)));
    tune_checkpoint(&server, 50, 0, true);

    let room_a = ConversationLocator::new(TEST_PLATFORM, "room-a");
    let room_b = ConversationLocator::new(TEST_PLATFORM, "room-b");
    // Call 0: room A's turn. Call 1: room A's flush, driven by room B opening. Call 2: room B's turn.
    let model = ScriptedModel::new([
        Completion::Reply("noted, all three".to_owned()),
        Completion::Reply("flushed room A".to_owned()),
        Completion::Reply("morning".to_owned()),
    ]);

    server
        .platform()
        .route_message(
            &model,
            &room_a,
            &PersonId::new(TEST_PLATFORM, "dave"),
            SUBSTANTIVE,
            &[PersonId::new(TEST_PLATFORM, "dave")],
        )
        .await
        .unwrap();
    let a_conversation = conversation_of(&server, &room_a);
    let a_session = server
        .sessions
        .get(a_conversation)
        .expect("room A is live")
        .id;

    // A fresh conversation opens. Its `ensure_session` sweeps room A first, so room A's working state
    // is durable before room B's first turn runs.
    server
        .platform()
        .route_message(
            &model,
            &room_b,
            &PersonId::new(TEST_PLATFORM, "erin"),
            "good morning team",
            &[PersonId::new(TEST_PLATFORM, "erin")],
        )
        .await
        .unwrap();

    // Exactly one flush landed — room A's — and its turn precedes room B's inbound in the log, so the
    // flush completed before room B's turn began.
    let flush_seqs = flush_turn_seqs(&server);
    assert_eq!(
        flush_seqs.len(),
        1,
        "room A flushed exactly once on room B's open"
    );
    let b_inbound = participant_turn_seq(&server, "good morning team");
    assert!(
        flush_seqs[0] < b_inbound,
        "room A's flush ({:?}) must precede room B's first turn ({:?})",
        flush_seqs[0],
        b_inbound
    );

    // Room A's session stayed open — the checkpoint rides the buffer, it does not end the session — and
    // its flush watermark advanced (the flush turn is its own, later, session-open).
    assert!(
        !server
            .control()
            .events()
            .unwrap()
            .iter()
            .any(|event| matches!(event.payload, EventPayload::SessionEnded { .. })),
        "the session-open checkpoint must leave every session open"
    );
    let still_live = server
        .sessions
        .get(a_conversation)
        .expect("room A stays live");
    assert_eq!(still_live.id, a_session, "room A's session is unchanged");
}

#[tokio::test]
async fn reuse_within_the_idle_gap_skips_the_open_sweep() {
    let server = born_server(ManualClock::new(Timestamp::from_millis(1_000)));
    tune_checkpoint(&server, 50, 0, true);

    let room_a = ConversationLocator::new(TEST_PLATFORM, "room-a");
    let room_b = ConversationLocator::new(TEST_PLATFORM, "room-b");
    // Call 0: room A's greeting (below substance). Call 1: room B's substantive turn. Call 2: room A's
    // reused turn. No flush is scripted: if the reuse path swept, room B (now substantive) would flush
    // and the model would be asked for a fourth, unscripted, completion.
    let model = ScriptedModel::new([
        Completion::Reply("hello dave".to_owned()),
        Completion::Reply("noted room B".to_owned()),
        Completion::Reply("still here".to_owned()),
    ]);

    // Room A opens light; room B opens substantive (room A is below substance, so B's open sweeps
    // nothing).
    server
        .platform()
        .route_message(
            &model,
            &room_a,
            &PersonId::new(TEST_PLATFORM, "dave"),
            "hi",
            &[PersonId::new(TEST_PLATFORM, "dave")],
        )
        .await
        .unwrap();
    server
        .platform()
        .route_message(
            &model,
            &room_b,
            &PersonId::new(TEST_PLATFORM, "erin"),
            SUBSTANTIVE,
            &[PersonId::new(TEST_PLATFORM, "erin")],
        )
        .await
        .unwrap();

    // Room A is messaged again within the idle gap: the pre-check sees a live session and reuses it, so
    // no open sweep runs — room B is not flushed even though its delta would clear the substance gate.
    server
        .platform()
        .route_message(
            &model,
            &room_a,
            &PersonId::new(TEST_PLATFORM, "dave"),
            "anything new?",
            &[PersonId::new(TEST_PLATFORM, "dave")],
        )
        .await
        .unwrap();

    assert!(
        flush_turn_seqs(&server).is_empty(),
        "a reuse open must not sweep the other live rooms"
    );
    assert_eq!(
        model.recorded_messages().len(),
        3,
        "no extra flush call was made"
    );
}

#[tokio::test]
async fn a_thin_delta_is_not_flushed_on_open() {
    let server = born_server(ManualClock::new(Timestamp::from_millis(1_000)));
    tune_checkpoint(&server, 5_000, 0, true);

    let room_a = ConversationLocator::new(TEST_PLATFORM, "room-a");
    let room_b = ConversationLocator::new(TEST_PLATFORM, "room-b");
    let model = ScriptedModel::new([
        Completion::Reply("noted".to_owned()),
        Completion::Reply("morning".to_owned()),
    ]);

    // Room A accrues a real message, but the substance threshold is set far above it, so room B's open
    // finds nothing worth a flush.
    server
        .platform()
        .route_message(
            &model,
            &room_a,
            &PersonId::new(TEST_PLATFORM, "dave"),
            SUBSTANTIVE,
            &[PersonId::new(TEST_PLATFORM, "dave")],
        )
        .await
        .unwrap();
    server
        .platform()
        .route_message(
            &model,
            &room_b,
            &PersonId::new(TEST_PLATFORM, "erin"),
            "good morning",
            &[PersonId::new(TEST_PLATFORM, "erin")],
        )
        .await
        .unwrap();

    assert!(
        flush_turn_seqs(&server).is_empty(),
        "a sub-threshold delta must not flush on open"
    );
}

#[tokio::test]
async fn flush_on_open_disabled_flushes_nothing() {
    let server = born_server(ManualClock::new(Timestamp::from_millis(1_000)));
    // Substance would pass and cooldown is zero, but the open trigger is switched off.
    tune_checkpoint(&server, 50, 0, false);

    let room_a = ConversationLocator::new(TEST_PLATFORM, "room-a");
    let room_b = ConversationLocator::new(TEST_PLATFORM, "room-b");
    let model = ScriptedModel::new([
        Completion::Reply("noted, all three".to_owned()),
        Completion::Reply("morning".to_owned()),
    ]);

    server
        .platform()
        .route_message(
            &model,
            &room_a,
            &PersonId::new(TEST_PLATFORM, "dave"),
            SUBSTANTIVE,
            &[PersonId::new(TEST_PLATFORM, "dave")],
        )
        .await
        .unwrap();
    server
        .platform()
        .route_message(
            &model,
            &room_b,
            &PersonId::new(TEST_PLATFORM, "erin"),
            "good morning",
            &[PersonId::new(TEST_PLATFORM, "erin")],
        )
        .await
        .unwrap();

    assert!(
        flush_turn_seqs(&server).is_empty(),
        "with flush_on_open off, opening a conversation sweeps nothing"
    );
}

#[tokio::test]
async fn a_post_idle_reopen_flushes_the_conversation_itself_via_the_close_not_the_sweep() {
    let clock = ManualClock::new(Timestamp::from_millis(1_000));
    let server = born_server(clock.clone());
    // A low substance threshold and a low flush floor, so the lapsed session's end-flush runs.
    tune_checkpoint(&server, 50, 0, true);
    let mut settings = server.control().settings().unwrap();
    settings.compaction.flush_min_turns = 2;
    server.control().set_settings(settings).unwrap();

    let room_a = ConversationLocator::new(TEST_PLATFORM, "room-a");
    // Calls 0 and 1: room A's two turns. Call 2: the lapsed session's end-flush (`flush_and_end`).
    // Call 3: the reopened session's turn.
    let model = ScriptedModel::new([
        Completion::Reply("noted one".to_owned()),
        Completion::Reply("noted two".to_owned()),
        Completion::Reply("flushed on close".to_owned()),
        Completion::Reply("welcome back".to_owned()),
    ]);

    server
        .platform()
        .route_message(
            &model,
            &room_a,
            &PersonId::new(TEST_PLATFORM, "dave"),
            SUBSTANTIVE,
            &[PersonId::new(TEST_PLATFORM, "dave")],
        )
        .await
        .unwrap();
    server
        .platform()
        .route_message(
            &model,
            &room_a,
            &PersonId::new(TEST_PLATFORM, "dave"),
            "and one more thing",
            &[PersonId::new(TEST_PLATFORM, "dave")],
        )
        .await
        .unwrap();

    // Cross the idle gap, then message room A again: its own lapsed session is flush-and-ended under
    // the lifecycle lock, and the session-open sweep skips room A itself (it is the opener) — so there
    // is exactly one flush (the close's) and exactly one SessionEnded, not a second sweep-driven flush.
    clock.advance_millis(
        server
            .control()
            .settings()
            .unwrap()
            .compaction
            .idle_gap_seconds
            * 1_000
            + 1_000,
    );
    server
        .platform()
        .route_message(
            &model,
            &room_a,
            &PersonId::new(TEST_PLATFORM, "dave"),
            "back now",
            &[PersonId::new(TEST_PLATFORM, "dave")],
        )
        .await
        .unwrap();

    assert_eq!(
        flush_turn_seqs(&server).len(),
        1,
        "the lapsed session is flushed once, by its close — not also by the open sweep"
    );
    let ends = server
        .control()
        .events()
        .unwrap()
        .iter()
        .filter(|event| matches!(event.payload, EventPayload::SessionEnded { .. }))
        .count();
    assert_eq!(ends, 1, "the lapsed session ended exactly once");
}

#[tokio::test]
async fn a_session_open_waives_the_cooldown_a_timer_sweep_would_enforce() {
    let server = born_server(ManualClock::new(Timestamp::from_millis(1_000)));
    // A long cooldown blocks the timer sweep; the open trigger waives it. `flush_on_open` is off so
    // room B's own open does not disturb room A — the two triggers are compared directly below.
    tune_checkpoint(&server, 50, 3_600, false);

    let room_a = ConversationLocator::new(TEST_PLATFORM, "room-a");
    let room_b = ConversationLocator::new(TEST_PLATFORM, "room-b");
    // Call 0: room A's substantive turn. Call 1: room B's greeting (the audience). Call 2: room A's
    // flush, driven only by the session-open trigger below.
    let model = ScriptedModel::new([
        Completion::Reply("noted, all three".to_owned()),
        Completion::Reply("hi".to_owned()),
        Completion::Reply("flushed room A".to_owned()),
    ]);

    server
        .platform()
        .route_message(
            &model,
            &room_a,
            &PersonId::new(TEST_PLATFORM, "dave"),
            SUBSTANTIVE,
            &[PersonId::new(TEST_PLATFORM, "dave")],
        )
        .await
        .unwrap();
    server
        .platform()
        .route_message(
            &model,
            &room_b,
            &PersonId::new(TEST_PLATFORM, "erin"),
            "hi there",
            &[PersonId::new(TEST_PLATFORM, "erin")],
        )
        .await
        .unwrap();

    // The timer sweep sees substance and a live audience (room B), but room A is younger than the
    // cooldown, so it blocks — no flush.
    assert_eq!(
        server
            .checkpoint_live_sessions(&model, CheckpointTrigger::Timer)
            .await
            .unwrap(),
        0,
        "the timer sweep is blocked by the cooldown"
    );

    // A fresh conversation opening waives the cooldown (and the audience), so the same delta flushes.
    let opener = ConversationId::generate();
    assert_eq!(
        server
            .checkpoint_live_sessions(&model, CheckpointTrigger::SessionOpen(opener))
            .await
            .unwrap(),
        1,
        "a session open waives the cooldown the timer enforces"
    );
    assert_eq!(
        flush_turn_seqs(&server).len(),
        1,
        "room A flushed on the open trigger"
    );
}
