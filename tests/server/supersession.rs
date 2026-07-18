//! Per-conversation turn supersession (issue #83): a batch arriving while a turn is generating
//! cooperatively cancels it, and the newer batch's turn answers once with everything in context.
//! These integration tests pin the observable semantics — the superseded turn returns
//! [`TurnOutcome::Superseded`] with its participant turns still durable, records no agent
//! `ConversationTurn`, leaves a capture-gated `ModelCallAborted` with [`SUPERSEDED_CAUSE`] when the
//! cancellation lands mid-stream, publishes an `Abandoned` progress frame, keeps committed Lua blocks
//! durable, measures the window from the burst's first unanswered arrival, and disables entirely at a
//! zero window — all against the real platform path with a gate-model fake in the `ConcurrencyProbe`
//! style.

use std::{collections::VecDeque, time::Duration};

use futures_util::stream::{self, StreamExt as _};
use tokio::{
    sync::{Notify, watch},
    time::timeout,
};
use zuihitsu::{
    Event, GenerateDelta, GenerateRequest, GenerateResponse, GenerateStream, Message, MessageInput,
    Settings, TurnId,
    event::SUPERSEDED_CAUSE,
    progress::{ProgressKind, TurnProgress},
};

use super::*;

/// Distinctive substrings planted in each batch's text, so a recorded prompt can be searched for
/// which messages a generation saw. A turn is the burst's winner exactly when its prompt carries
/// *every* marker — only the last turn to run has every prior batch's participant turn replayed plus
/// its own inbound, so this identifies the winner regardless of how the concurrent arrivals raced.
const FIRST_MARK: &str = "VENUE-QUERY-4471";
const SECOND_MARK: &str = "CORRECTION-8213";
const THIRD_MARK: &str = "TRAIN-UPDATE-6650";
const FIRST_TEXT: &str = "Can you summarise where we landed on the venue? (VENUE-QUERY-4471)";
const SECOND_TEXT: &str = "Actually, scratch that — CORRECTION-8213: the venue moved to the wharf.";
const THIRD_TEXT: &str =
    "One more thing — TRAIN-UPDATE-6650: push the booking to 8:30, my train's late.";
/// The self-observation a committed block writes in the preserved-blocks test.
const BLOCK_MARK: &str = "SUPERSEDE-BLOCK-9931";
/// An upper bound on every condition wait: the tests are condition-driven (a watch, a join), and this
/// only bounds a genuine hang so a broken build fails fast rather than blocking.
const WAIT: Duration = Duration::from_secs(10);

/// A gate model in the `ConcurrencyProbe` style, built to race the supersession select. A generation
/// whose prompt carries every marker is the burst winner and completes immediately; any other
/// generation either commits a scripted block (no pause) or *parks inside the stream* — yielding one
/// reply fragment, then holding the stream open on a [`Notify`] without ever yielding a terminal — so
/// the in-flight turn sits in `generate_streaming`'s mid-stream select and a newer batch's arrival can
/// win it. A parked generation resumes only when the test releases it (the window and zero-window
/// tests, where the turn must run to completion) or is dropped when the turn is superseded.
struct GateModel {
    /// Every marker the winning batch's prompt carries; a generation replies immediately iff its
    /// prompt contains all of them.
    markers: Vec<String>,
    /// The reply the winner posts.
    marker_reply: String,
    /// The reply a parked generation posts once released.
    parked_reply: String,
    /// Immediate `run_lua` tool calls for non-winner generations, consumed in call order; an empty
    /// script means the generation parks.
    script: std::sync::Mutex<VecDeque<String>>,
    /// The count of generations that have entered the park branch — the signal a test waits on to
    /// know a turn is genuinely mid-generation before it sends the interrupting batch.
    entered: watch::Sender<usize>,
    /// Released to let a parked generation complete with [`GateModel::parked_reply`]. An `Arc` so the
    /// parked stream (which must be `'static`) owns a handle rather than borrowing the model.
    release: std::sync::Arc<Notify>,
    /// Each `generate_stream` call's request messages, in order — the "everything in context" witness.
    seen: std::sync::Mutex<Vec<Vec<Message>>>,
}

impl GateModel {
    fn new(markers: &[&str]) -> std::sync::Arc<GateModel> {
        let (entered, _rx) = watch::channel(0usize);
        std::sync::Arc::new(GateModel {
            markers: markers.iter().map(|m| (*m).to_owned()).collect(),
            marker_reply: "Got it — I'll fold that in and reply once.".to_owned(),
            parked_reply: "Here's the summary you asked for.".to_owned(),
            script: std::sync::Mutex::new(VecDeque::new()),
            entered,
            release: std::sync::Arc::new(Notify::new()),
            seen: std::sync::Mutex::new(Vec::new()),
        })
    }

    fn entered_rx(&self) -> watch::Receiver<usize> {
        self.entered.subscribe()
    }
}

#[async_trait::async_trait]
impl ModelClient for GateModel {
    fn model_id(&self) -> &str {
        "gate-model"
    }

    async fn generate_stream(&self, request: &GenerateRequest) -> GenerateStream {
        self.seen.lock().unwrap().push(request.messages.clone());
        let prompt = joined(&request.messages);
        if self.markers.iter().all(|marker| prompt.contains(marker)) {
            return stream_response(Ok(reply(&self.marker_reply)));
        }
        if let Some(script) = self.script.lock().unwrap().pop_front() {
            return stream_response(Ok(tool_call(&script)));
        }
        // Park inside the stream: yield one fragment so the turn has a partial to abandon, then hold
        // the stream open with no terminal until released or the turn is superseded out from under it.
        self.entered.send_modify(|count| *count += 1);
        let reply_text = self.parked_reply.clone();
        let release = self.release.clone();
        // Yield one reply fragment, then hold the stream open on the release notify with no terminal —
        // the mid-stream gap the supersession select races against. When released, the terminal lands.
        let fragment = stream::once(async {
            Ok::<GenerateDelta, ModelError>(GenerateDelta::Reply(
                "Let me pull that together. ".to_owned(),
            ))
        });
        let terminal = stream::once(async move {
            release.notified().await;
            Ok(GenerateDelta::Finished(reply(&reply_text)))
        });
        Box::pin(fragment.chain(terminal))
    }
}

fn reply(text: &str) -> GenerateResponse {
    GenerateResponse {
        completion: Completion::Reply(text.to_owned()),
        usage: Usage::default(),
        reasoning: None,
        finish_reason: Some("stop".to_owned()),
    }
}

fn tool_call(script: &str) -> GenerateResponse {
    GenerateResponse {
        completion: Completion::ToolCalls(vec![ToolCall {
            id: "lua".to_owned(),
            name: "run_lua".to_owned(),
            arguments: serde_json::json!({ "script": script }).to_string(),
        }]),
        usage: Usage::default(),
        reasoning: None,
        finish_reason: Some("stop".to_owned()),
    }
}

fn joined(messages: &[Message]) -> String {
    messages
        .iter()
        .map(|message| message.content.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Deliver one batch as its own concurrent turn, returning its outcome and participant turn ids. Not
/// spawned as a shared borrow (the instance is not `Clone`): each task owns an `Arc` clone.
fn spawn_batch(
    server: std::sync::Arc<Server>,
    gate: std::sync::Arc<GateModel>,
    text: &'static str,
) -> tokio::task::JoinHandle<(TurnOutcome, Vec<String>)> {
    tokio::spawn(async move {
        let leads = ConversationLocator::new(TEST_PLATFORM, "leads");
        let dave = PersonId::new(TEST_PLATFORM, "dave");
        let response = server
            .platform()
            .route_messages(
                gate.as_ref(),
                &leads,
                &[MessageInput {
                    sender: dave.clone(),
                    text: text.to_owned(),
                }],
                std::slice::from_ref(&dave),
            )
            .await
            .expect("route_messages succeeds");
        (response.outcome, response.participant_turn_ids)
    })
}

fn agent_turn(event: &Event) -> Option<(TurnId, String)> {
    match &event.payload {
        EventPayload::ConversationTurn {
            role: TurnRole::Agent,
            turn_id,
            text,
            ..
        } => Some((*turn_id, text.clone())),
        _ => None,
    }
}

fn participant_turn_id(event: &Event) -> Option<String> {
    match &event.payload {
        EventPayload::ConversationTurn {
            role: TurnRole::Participant,
            turn_id,
            ..
        } => Some(turn_id.0.to_string()),
        _ => None,
    }
}

fn superseded_abort(event: &Event) -> Option<TurnId> {
    match &event.payload {
        EventPayload::ModelCallAborted { turn_id, cause, .. } if cause == SUPERSEDED_CAUSE => {
            Some(*turn_id)
        }
        _ => None,
    }
}

/// Await the gate's park signal, so the test knows a turn is genuinely mid-generation (past the
/// run-entry supersession check) before it sends the batch that should cancel it.
async fn await_parked(gate: &GateModel, count: usize) {
    let mut rx = gate.entered_rx();
    timeout(WAIT, rx.wait_for(|entered| *entered >= count))
        .await
        .expect("a generation reaches the park boundary")
        .expect("the gate's entered watch stays open");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_second_batch_supersedes_the_in_flight_turn() {
    let (server, _clock) = born_agent();
    let server = std::sync::Arc::new(server);
    let gate = GateModel::new(&[FIRST_MARK, SECOND_MARK]);

    let first = spawn_batch(server.clone(), gate.clone(), FIRST_TEXT);
    await_parked(&gate, 1).await;
    let second = spawn_batch(server.clone(), gate.clone(), SECOND_TEXT);

    let (outcome1, ids1) = timeout(WAIT, first).await.unwrap().unwrap();
    let (outcome2, ids2) = timeout(WAIT, second).await.unwrap().unwrap();

    assert_eq!(outcome1, TurnOutcome::Superseded);
    assert!(
        !ids1.is_empty(),
        "the superseded turn still carries its participant turn id"
    );
    assert!(matches!(outcome2, TurnOutcome::Reply(_)));

    let events = server.control().events().unwrap();
    let agent_turns: Vec<_> = events.iter().filter_map(agent_turn).collect();
    assert_eq!(
        agent_turns.len(),
        1,
        "only the winner records an agent turn; the loser records none"
    );

    // Both batches' participant turns are durable and returned to their callers.
    let participant_ids: Vec<String> = events.iter().filter_map(participant_turn_id).collect();
    assert_eq!(participant_ids.len(), 2);
    for id in ids1.iter().chain(ids2.iter()) {
        assert!(
            participant_ids.contains(id),
            "participant turn {id} is durable in the log"
        );
    }

    // Exactly one supersession abort, on the loser's (dead) agent turn id — not the winner's.
    let aborts: Vec<TurnId> = events.iter().filter_map(superseded_abort).collect();
    assert_eq!(aborts.len(), 1, "one mid-stream cancellation was recorded");
    assert_ne!(
        aborts[0], agent_turns[0].0,
        "the abort belongs to the loser, not the winner"
    );

    // The winner saw both messages in context.
    let winner_prompt = gate
        .seen
        .lock()
        .unwrap()
        .iter()
        .map(|messages| joined(messages))
        .find(|prompt| prompt.contains(SECOND_MARK))
        .expect("the winner's request is recorded");
    assert!(
        winner_prompt.contains(FIRST_MARK) && winner_prompt.contains(SECOND_MARK),
        "the winner's prompt carries both the first message and the correction"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_superseded_generation_ends_with_an_abandoned_frame_on_its_own_turn() {
    let (server, _clock) = born_agent();
    let server = std::sync::Arc::new(server);
    let gate = GateModel::new(&[FIRST_MARK, SECOND_MARK]);
    let mut frames = server.subscribe_progress();

    let first = spawn_batch(server.clone(), gate.clone(), FIRST_TEXT);
    await_parked(&gate, 1).await;
    let second = spawn_batch(server.clone(), gate.clone(), SECOND_TEXT);

    let (outcome1, _) = timeout(WAIT, first).await.unwrap().unwrap();
    let (outcome2, _) = timeout(WAIT, second).await.unwrap().unwrap();
    assert_eq!(outcome1, TurnOutcome::Superseded);
    assert!(matches!(outcome2, TurnOutcome::Reply(_)));

    let mut received: Vec<TurnProgress> = Vec::new();
    while let Ok(frame) = frames.try_recv() {
        received.push(frame);
    }

    let abandoned_at = received
        .iter()
        .position(|frame| frame.kind == ProgressKind::Abandoned)
        .expect("the superseded generation ends with an abandoned frame");
    assert_eq!(
        received
            .iter()
            .filter(|frame| frame.kind == ProgressKind::Abandoned)
            .count(),
        1,
        "exactly one abandonment — the loser's"
    );
    let loser = received[abandoned_at].turn_id;
    assert_eq!(received[abandoned_at].text, SUPERSEDED_CAUSE);

    // The winner's reply frames land after the abandonment, on a different turn.
    let (winner_at, winner) = received
        .iter()
        .enumerate()
        .find(|(_, frame)| frame.kind == ProgressKind::Reply && frame.turn_id != loser)
        .expect("the winner streams its reply");
    assert!(
        winner_at > abandoned_at,
        "the winner's frames follow the loser's abandonment"
    );
    assert_ne!(winner.turn_id, loser);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_chain_of_batches_supersedes_to_the_newest() {
    let (server, _clock) = born_agent();
    let server = std::sync::Arc::new(server);
    // Only the third batch's turn ends up carrying all three markers, so it is the sole winner: the
    // first and second turns each park mid-generation and are superseded in turn. Each link in the
    // chain is gated on the previous turn genuinely parking — an unsequenced burst would let the
    // second and third arrivals race, and a second turn that replays the third's already-durable
    // inbound sees every marker and answers as the winner itself.
    let gate = GateModel::new(&[FIRST_MARK, SECOND_MARK, THIRD_MARK]);

    let first = spawn_batch(server.clone(), gate.clone(), FIRST_TEXT);
    await_parked(&gate, 1).await;
    let second = spawn_batch(server.clone(), gate.clone(), SECOND_TEXT);
    await_parked(&gate, 2).await;
    let third = spawn_batch(server.clone(), gate.clone(), THIRD_TEXT);

    let (outcome1, _) = timeout(WAIT, first).await.unwrap().unwrap();
    let (outcome2, _) = timeout(WAIT, second).await.unwrap().unwrap();
    let (outcome3, _) = timeout(WAIT, third).await.unwrap().unwrap();

    assert_eq!(outcome1, TurnOutcome::Superseded);
    assert_eq!(outcome2, TurnOutcome::Superseded);
    assert!(matches!(outcome3, TurnOutcome::Reply(_)));

    let events = server.control().events().unwrap();
    assert_eq!(
        events.iter().filter_map(agent_turn).count(),
        1,
        "only the newest batch answers the whole burst"
    );

    let winner_prompt = gate
        .seen
        .lock()
        .unwrap()
        .iter()
        .map(|messages| joined(messages))
        .find(|prompt| prompt.contains(THIRD_MARK))
        .expect("the winner's request is recorded");
    assert!(
        [FIRST_MARK, SECOND_MARK, THIRD_MARK]
            .iter()
            .all(|mark| winner_prompt.contains(mark)),
        "the winner's prompt carries all three messages"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_batch_beyond_the_window_waits_instead_of_superseding() {
    let (server, clock) = born_agent();
    let server = std::sync::Arc::new(server);
    let gate = GateModel::new(&[FIRST_MARK, SECOND_MARK]);

    let first = spawn_batch(server.clone(), gate.clone(), FIRST_TEXT);
    await_parked(&gate, 1).await;

    // Move past the default 60s window while turn #1 is mid-generation: a later batch can no longer
    // cancel it, so it must run to completion and the newcomer queues behind it.
    clock.advance_millis(61 * 1_000);

    let second = spawn_batch(server.clone(), gate.clone(), SECOND_TEXT);
    // Let the (uncancellable) in-flight turn finish; turn #2 then takes the slot behind it.
    gate.release.notify_one();

    let (outcome1, _) = timeout(WAIT, first).await.unwrap().unwrap();
    let (outcome2, _) = timeout(WAIT, second).await.unwrap().unwrap();
    assert!(
        matches!(outcome1, TurnOutcome::Reply(_)),
        "the in-flight turn runs to completion past the window, got {outcome1:?}"
    );
    assert!(matches!(outcome2, TurnOutcome::Reply(_)));

    let events = server.control().events().unwrap();
    let agent_turns: Vec<_> = events.iter().filter_map(agent_turn).collect();
    assert_eq!(
        agent_turns.len(),
        2,
        "both turns answer; neither is superseded"
    );
    // Serialization order: the in-flight turn's reply precedes the queued one's.
    assert_eq!(agent_turns[0].1, gate.parked_reply);
    assert_eq!(agent_turns[1].1, gate.marker_reply);
    assert!(
        events.iter().filter_map(superseded_abort).next().is_none(),
        "no supersession abort past the window"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn supersession_preserves_committed_blocks() {
    let (server, _clock) = born_agent();
    let server = std::sync::Arc::new(server);
    let gate = GateModel::new(&[FIRST_MARK, SECOND_MARK]);
    // Turn #1's first generation commits a durable memory, then its second generation parks and is
    // superseded — so the block commits before the cancellation lands.
    gate.script.lock().unwrap().push_back(format!(
        r#"memory.create("topic/venue", "{BLOCK_MARK} the venue moved to the wharf")"#
    ));

    let first = spawn_batch(server.clone(), gate.clone(), FIRST_TEXT);
    // The park signal only fires on the *second* generation, so reaching it proves the block committed.
    await_parked(&gate, 1).await;
    let second = spawn_batch(server.clone(), gate.clone(), SECOND_TEXT);

    let (outcome1, _) = timeout(WAIT, first).await.unwrap().unwrap();
    let (outcome2, _) = timeout(WAIT, second).await.unwrap().unwrap();
    assert_eq!(outcome1, TurnOutcome::Superseded);
    assert!(matches!(outcome2, TurnOutcome::Reply(_)));

    let events = server.control().events().unwrap();
    let agent_turns: Vec<_> = events.iter().filter_map(agent_turn).collect();
    assert_eq!(agent_turns.len(), 1, "the winner answered once");

    // The committed block survived the supersession: its `LuaExecuted` and memory write are durable,
    // orphaned under the loser's (dead) turn id rather than the winner's.
    let lua_turn = events
        .iter()
        .find_map(|event| match &event.payload {
            EventPayload::LuaExecuted { turn_id, .. } => Some(*turn_id),
            _ => None,
        })
        .expect("the committed block's LuaExecuted is durable");
    let abort_turn = events
        .iter()
        .find_map(superseded_abort)
        .expect("the loser's supersession abort is recorded");
    assert_eq!(
        lua_turn, abort_turn,
        "the block is orphaned under the superseded turn's id"
    );
    assert_ne!(
        lua_turn, agent_turns[0].0,
        "the block is not attributed to the winner's turn"
    );
    assert!(
        events.iter().any(|event| matches!(
            &event.payload,
            EventPayload::MemoryContentAppended { text, .. } if text.contains(BLOCK_MARK)
        )),
        "the block's memory write is durable"
    );

    // A fresh read after the supersession still sees the committed memory — the block is not rolled
    // back with the abandoned turn. (Buffer replay deliberately orphans it, matching the `Deferred`
    // shape, so it is durable in the log without re-entering the successor's replayed prompt.)
    assert!(
        server.control().memory("topic/venue").unwrap().is_some(),
        "the memory the committed block created survives the supersession"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_zero_window_disables_supersession() {
    let (server, _clock) = born_agent();
    let server = std::sync::Arc::new(server);
    // Window 0: per-conversation serialization stays on, but no turn is ever cancelled.
    let mut settings: Settings = server.control().settings().unwrap();
    settings.turn.supersede_window_seconds = 0;
    server.control().set_settings(settings).unwrap();

    let gate = GateModel::new(&[FIRST_MARK, SECOND_MARK]);

    let first = spawn_batch(server.clone(), gate.clone(), FIRST_TEXT);
    await_parked(&gate, 1).await;
    let second = spawn_batch(server.clone(), gate.clone(), SECOND_TEXT);
    // The in-flight turn cannot be superseded at a zero window; release it so it completes normally.
    gate.release.notify_one();

    let (outcome1, _) = timeout(WAIT, first).await.unwrap().unwrap();
    let (outcome2, _) = timeout(WAIT, second).await.unwrap().unwrap();
    assert!(
        matches!(outcome1, TurnOutcome::Reply(_)),
        "a zero window leaves the in-flight turn uncancellable, got {outcome1:?}"
    );
    assert!(matches!(outcome2, TurnOutcome::Reply(_)));

    let events = server.control().events().unwrap();
    assert_eq!(
        events.iter().filter_map(agent_turn).count(),
        2,
        "both turns answer; serialization holds without cancellation"
    );
    assert!(
        events.iter().filter_map(superseded_abort).next().is_none(),
        "no supersession abort at a zero window"
    );
}
