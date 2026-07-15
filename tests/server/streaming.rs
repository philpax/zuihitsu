//! Token streaming's load-bearing invariant: a streamed turn records exactly the events an unary
//! one does. The progress frames a live viewer watches are ephemeral — never stored, never
//! replayed — so the log is a pure function of the conversation regardless of who was watching.

use std::sync::atomic::{AtomicU32, Ordering as AtomicOrdering};

use zuihitsu::{
    PersonId, Seq,
    progress::{ProgressKind, TurnProgress},
};

use super::*;

/// The comparable face of an event: its payload kind, plus — for the two payloads a streamed
/// generation could plausibly distort — the recorded substance. Ids and timestamps differ across
/// two servers by construction (ULIDs are random), so equality is over what the turn *said and
/// recorded*, not the identifiers it minted.
fn comparable(events: &[zuihitsu::Event]) -> Vec<String> {
    events
        .iter()
        .map(|event| match &event.payload {
            EventPayload::ModelCalled {
                completion,
                reasoning,
                finish_reason,
                usage,
                phase,
                ..
            } => format!(
                "ModelCalled {phase:?} {completion:?} reasoning={reasoning:?} finish={finish_reason:?} usage={usage:?}"
            ),
            EventPayload::ConversationTurn { role, text, .. } => {
                format!("ConversationTurn {role:?} {text:?}")
            }
            payload => payload.kind().to_owned(),
        })
        .collect()
}

fn deliberation_script() -> ScriptedModel {
    ScriptedModel::with_deliberation([(
        Completion::Reply("Hello there, and welcome back.".to_owned()),
        "The participant greeted me; a warm reply suits.".to_owned(),
        Usage {
            prompt_tokens: Some(120),
            completion_tokens: Some(9),
            total_tokens: Some(129),
            ..Usage::default()
        },
    )])
}

async fn run_one_turn(server: &Server, model: &dyn ModelClient) {
    server.control().create_agent(&seed()).unwrap();
    server
        .platform()
        .route_message(
            model,
            &ConversationLocator::new(TEST_PLATFORM, "general"),
            &PersonId::new(TEST_PLATFORM, "dave"),
            "hello again",
            &[PersonId::new(TEST_PLATFORM, "dave")],
        )
        .await
        .unwrap();
}

/// The invariant test: one server runs the turn unary (nobody watching), the other streams the
/// same scripted deliberation fragment by fragment to a held progress subscription. The recorded
/// event sequences must be identical in kind and substance.
#[tokio::test]
async fn a_streamed_turn_records_the_same_events_as_an_unary_one() {
    let unary = Server::in_memory(clock()).unwrap();
    run_one_turn(&unary, &deliberation_script()).await;

    let streamed = Server::in_memory(clock()).unwrap();
    // Every turn streams; a held subscription receives the frames as the turn runs.
    let mut frames = streamed.subscribe_progress();
    run_one_turn(&streamed, &deliberation_script()).await;

    let unary_events = unary.control().events_from(Seq(0)).unwrap();
    let streamed_events = streamed.control().events_from(Seq(0)).unwrap();
    assert_eq!(comparable(&unary_events), comparable(&streamed_events));

    // The watcher saw the whole deliberation, in order, reassembling exactly the recorded text.
    let mut reasoning = String::new();
    let mut reply = String::new();
    let mut received: Vec<TurnProgress> = Vec::new();
    while let Ok(frame) = frames.try_recv() {
        received.push(frame);
    }
    assert!(
        !received.is_empty(),
        "the watched turn published no progress frames"
    );
    for frame in &received {
        assert_eq!(frame.step, 0, "one generation, so every frame is step 0");
        match frame.kind {
            ProgressKind::Reasoning => reasoning.push_str(&frame.text),
            ProgressKind::Reply => reply.push_str(&frame.text),
            ProgressKind::Restart => panic!("a scripted run never restarts"),
            ProgressKind::Abandoned => panic!("a scripted run never abandons"),
        }
    }
    assert_eq!(reasoning, "The participant greeted me; a warm reply suits.");
    assert_eq!(reply, "Hello there, and welcome back.");
}

/// With no subscriber the frames are published to nobody (free) and the outcome is identical —
/// watching is pure observation. Guarded by running the same script unwatched and asserting the
/// same recorded substance as the watched baseline.
#[tokio::test]
async fn an_unwatched_turn_records_identically() {
    let watched = Server::in_memory(clock()).unwrap();
    let _frames = watched.subscribe_progress();
    run_one_turn(&watched, &deliberation_script()).await;

    let unwatched = Server::in_memory(clock()).unwrap();
    run_one_turn(&unwatched, &deliberation_script()).await;

    assert_eq!(
        comparable(&watched.control().events_from(Seq(0)).unwrap()),
        comparable(&unwatched.control().events_from(Seq(0)).unwrap()),
    );
}

/// A stream that dies mid-generation surfaces as the model-error path the unary call would take:
/// the turn defers (the backend is unreachable), no partial text leaks into the log, and the feed
/// carries a terminal `abandoned` frame — a deferral records no agent `ConversationTurn`, so that
/// frame is a live viewer's only cue to drop the dead generation rather than show it forever.
#[tokio::test]
async fn a_stream_that_fails_midway_defers_the_turn_like_an_unary_failure() {
    struct DiesMidStream;

    #[async_trait::async_trait]
    impl ModelClient for DiesMidStream {
        fn model_id(&self) -> &str {
            "dies-mid-stream"
        }

        async fn generate_stream(&self, _: &GenerateRequest) -> zuihitsu::GenerateStream {
            Box::pin(futures_stream(vec![
                Ok(zuihitsu::GenerateDelta::Reply("Hel".to_owned())),
                Err(ModelError::Backend {
                    model: "dies-mid-stream".to_owned(),
                    message: "connection reset".to_owned(),
                    transient: true,
                }),
            ]))
        }
    }

    let server = Server::in_memory(clock()).unwrap();
    let mut frames = server.subscribe_progress();
    server.control().create_agent(&seed()).unwrap();
    let outcome = server
        .platform()
        .route_message(
            &DiesMidStream,
            &ConversationLocator::new(TEST_PLATFORM, "general"),
            &PersonId::new(TEST_PLATFORM, "dave"),
            "hello",
            &[PersonId::new(TEST_PLATFORM, "dave")],
        )
        .await
        .unwrap();
    assert!(
        matches!(outcome.outcome, TurnOutcome::Deferred),
        "a mid-stream transport failure defers like an unary one, got {outcome:?}"
    );
    let mut kinds = Vec::new();
    while let Ok(frame) = frames.try_recv() {
        kinds.push(frame.kind);
    }
    assert_eq!(
        kinds.last(),
        Some(&ProgressKind::Abandoned),
        "the dead generation ends with an abandoned frame, got {kinds:?}"
    );
    // No agent turn and no model call landed — the partial "Hel" is nowhere in the log.
    let events = server.control().events_from(Seq(0)).unwrap();
    assert!(events.iter().all(|event| {
        !matches!(&event.payload, EventPayload::ModelCalled { .. })
            && !matches!(
                &event.payload,
                EventPayload::ConversationTurn {
                    role: TurnRole::Agent,
                    ..
                }
            )
    }));
}

/// A plain iterator-backed delta stream for test fakes.
fn futures_stream(
    items: Vec<Result<zuihitsu::GenerateDelta, ModelError>>,
) -> impl futures_util::Stream<Item = Result<zuihitsu::GenerateDelta, ModelError>> + Send {
    futures_util::stream::iter(items)
}

/// The restart path end to end: attempt one streams fragments then dies transiently; the retry
/// wrapper discards it with a `Restarted` marker and re-drives; attempt two completes. The turn
/// succeeds, the discarded partial lands durably as one `ModelCallAborted` (attempt 1, with the
/// partial text), the successful attempt is the one `ModelCalled`, and the viewer saw a `restart`
/// frame voiding its accumulation.
#[tokio::test]
async fn a_mid_stream_failure_restarts_and_records_the_abort() {
    /// Fails after two reply fragments on the first attempt; streams the whole reply on the second.
    struct DiesOnceMidStream {
        attempts: AtomicU32,
    }

    #[async_trait::async_trait]
    impl ModelClient for DiesOnceMidStream {
        fn model_id(&self) -> &str {
            "dies-once"
        }

        async fn generate_stream(&self, _: &GenerateRequest) -> GenerateStream {
            let attempt = self.attempts.fetch_add(1, AtomicOrdering::SeqCst);
            if attempt == 0 {
                Box::pin(futures_util::stream::iter(vec![
                    Ok(zuihitsu::GenerateDelta::Reply("Hello ".to_owned())),
                    Ok(zuihitsu::GenerateDelta::Reply("there".to_owned())),
                    Err(ModelError::Backend {
                        model: "dies-once".to_owned(),
                        message: "connection reset".to_owned(),
                        transient: true,
                    }),
                ]))
            } else {
                stream_response(Ok(GenerateResponse {
                    completion: Completion::Reply("Hello there, and welcome.".to_owned()),
                    usage: Usage::default(),
                    reasoning: None,
                    finish_reason: Some("stop".to_owned()),
                }))
            }
        }
    }

    let server = Server::in_memory(clock()).unwrap();
    let mut frames = server.subscribe_progress();
    server.control().create_agent(&seed()).unwrap();
    let model = zuihitsu::RetryingModel::new(
        Arc::new(DiesOnceMidStream {
            attempts: AtomicU32::new(0),
        }),
        &zuihitsu::ResilienceConfig {
            max_attempts: 3,
            backoff_base_ms: 1,
            backoff_max_ms: 2,
            ..zuihitsu::ResilienceConfig::default()
        },
    );
    let outcome = server
        .platform()
        .route_message(
            &model,
            &ConversationLocator::new(TEST_PLATFORM, "general"),
            &PersonId::new(TEST_PLATFORM, "dave"),
            "hello",
            &[PersonId::new(TEST_PLATFORM, "dave")],
        )
        .await
        .unwrap();
    assert!(
        matches!(outcome.outcome, TurnOutcome::Reply(reply) if reply == "Hello there, and welcome.")
    );

    // Durable visibility: exactly one abort, carrying the discarded partial and its cause.
    let events = server.control().events_from(zuihitsu::Seq(0)).unwrap();
    let aborts: Vec<_> = events
        .iter()
        .filter_map(|event| match &event.payload {
            EventPayload::ModelCallAborted {
                attempt,
                cause,
                partial_reply,
                ..
            } => Some((*attempt, cause.clone(), partial_reply.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(aborts.len(), 1);
    assert_eq!(aborts[0].0, 1);
    assert!(aborts[0].1.contains("connection reset"));
    assert_eq!(aborts[0].2, "Hello there");
    // The successful attempt is the one ModelCalled, complete.
    let calls = events
        .iter()
        .filter(|event| matches!(&event.payload, EventPayload::ModelCalled { .. }))
        .count();
    assert_eq!(calls, 1);

    // The viewer saw the first attempt's fragments, then a restart voiding them.
    let mut kinds = Vec::new();
    while let Ok(frame) = frames.try_recv() {
        kinds.push(frame.kind);
    }
    assert!(kinds.contains(&ProgressKind::Restart));
}
