//! Backend-failure resilience at the routed-message boundary (spec §Event sourcing: retries the
//! agent never saw are infra-transparent — they emit nothing to the log). An unreachable model
//! defers the turn instead of erroring it: the inbound participant turn is durable and no agent
//! turn is recorded, so the next successful turn's buffer replay covers every deferred inbound in
//! one response cycle. Non-transient failures keep today's error path. Driven with the
//! fault-injecting `FlakyModel` under the `RetryingModel` wrapper — the same wiring the serving
//! host uses — so retries, the circuit breaker, and the deferral are exercised together.

mod common;

use std::sync::Arc;

use zuihitsu::{
    Completion, ConversationLocator, FlakyModel, InstanceError, ManualClock, ModelClient,
    ModelError, PersonId, PlatformResponse, ResilienceConfig, RetryingModel, ScriptedModel,
    SeedSelf, Seq, Server, TEST_PLATFORM, TurnError, TurnOutcome, TurnRole, event::EventPayload,
};

use common::time::test_now;

fn born_server() -> Server {
    let server = Server::in_memory(Box::new(ManualClock::new(test_now()))).unwrap();
    server
        .control()
        .create_agent(&SeedSelf {
            agent_name: "Kestrel".to_owned(),
            persona: "A thoughtful, discreet companion with a long memory.".to_owned(),
            seed_entries: vec![],
        })
        .unwrap();
    server
}

fn locator() -> ConversationLocator {
    ConversationLocator::new(TEST_PLATFORM, "general")
}

/// A resilience policy with instant backoff, so the retry loop runs in milliseconds; the breaker
/// threshold is high so only the tests that drive the circuit reach it.
fn tiny_resilience() -> ResilienceConfig {
    ResilienceConfig {
        request_timeout_seconds: 1,
        max_attempts: 3,
        backoff_base_ms: 1,
        backoff_max_ms: 2,
        breaker_failure_threshold: 100,
        breaker_open_seconds: 3_600,
    }
}

async fn route(
    server: &Server,
    model: &dyn ModelClient,
    text: &str,
) -> Result<PlatformResponse, InstanceError> {
    server
        .platform()
        .route_message(
            model,
            &locator(),
            &PersonId::new(TEST_PLATFORM, "dave"),
            text,
            &[PersonId::new(TEST_PLATFORM, "dave")],
        )
        .await
}

/// The conversation turns in the log, as `(role, text)` pairs — what the durability assertions read.
fn turns(server: &Server) -> Vec<(TurnRole, String)> {
    server
        .control()
        .events_from(Seq::ZERO)
        .unwrap()
        .into_iter()
        .filter_map(|event| match event.payload {
            EventPayload::ConversationTurn { role, text, .. } => Some((role, text)),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn an_unreachable_model_defers_the_turn_and_keeps_the_inbound() {
    let server = born_server();
    let flaky = Arc::new(FlakyModel::always_transient());
    let model = RetryingModel::new(flaky.clone(), &tiny_resilience());

    let outcome = route(&server, &model, "remember the launch moved to friday")
        .await
        .unwrap();
    assert_eq!(outcome.outcome, TurnOutcome::Deferred);
    assert_eq!(
        flaky.calls(),
        3,
        "the retry bound was exhausted before deferring"
    );

    let turns = turns(&server);
    assert!(
        turns
            .iter()
            .any(|(role, text)| *role == TurnRole::Participant
                && text == "remember the launch moved to friday"),
        "the inbound is durable despite the deferral"
    );
    assert!(
        !turns.iter().any(|(role, _)| *role == TurnRole::Agent),
        "a deferred cycle records no agent turn (infra-transparent)"
    );
}

#[tokio::test]
async fn the_next_successful_turn_covers_the_deferred_inbound() {
    let server = born_server();
    let dead = RetryingModel::new(Arc::new(FlakyModel::always_transient()), &tiny_resilience());
    let outcome = route(&server, &dead, "the launch moved to friday")
        .await
        .unwrap();
    assert_eq!(outcome.outcome, TurnOutcome::Deferred);

    // The backend recovers: the next message's turn replays the buffer, which carries the
    // deferred inbound — passive catch-up, one response cycle covering both messages.
    let recovered = ScriptedModel::new([Completion::Reply("Noted — Friday it is.".to_owned())]);
    let outcome = route(&server, &recovered, "did you get that?")
        .await
        .unwrap();
    assert_eq!(
        outcome.outcome,
        TurnOutcome::Reply("Noted — Friday it is.".to_owned())
    );
    assert!(
        recovered
            .recorded_messages()
            .iter()
            .flatten()
            .any(|message| message.content.contains("the launch moved to friday")),
        "the deferred inbound rode the buffer replay into the recovered turn"
    );
}

#[tokio::test]
async fn a_non_transient_failure_errors_the_turn_without_retry_or_deferral() {
    let server = born_server();
    let flaky = Arc::new(FlakyModel::always_permanent());
    let model = RetryingModel::new(flaky.clone(), &tiny_resilience());

    let error = route(&server, &model, "hello").await;
    assert!(
        matches!(
            error,
            Err(InstanceError::Turn {
                error: TurnError::Model(ModelError::Backend {
                    transient: false,
                    ..
                }),
                ..
            })
        ),
        "a non-transient failure keeps the error path: {error:?}"
    );
    assert_eq!(flaky.calls(), 1, "a non-transient failure is not retried");
}

#[tokio::test]
async fn an_open_circuit_defers_without_touching_the_backend() {
    let server = born_server();
    let flaky = Arc::new(FlakyModel::always_transient());
    let model = RetryingModel::new(
        flaky.clone(),
        &ResilienceConfig {
            max_attempts: 1,
            breaker_failure_threshold: 1,
            ..tiny_resilience()
        },
    );

    // The first message's single attempt fails and opens the circuit.
    assert_eq!(
        route(&server, &model, "first").await.unwrap().outcome,
        TurnOutcome::Deferred
    );
    assert_eq!(flaky.calls(), 1);

    // While open, the next message still lands (durable inbound, Deferred outcome) but fails fast
    // — the backend sees no call.
    assert_eq!(
        route(&server, &model, "second").await.unwrap().outcome,
        TurnOutcome::Deferred
    );
    assert_eq!(
        flaky.calls(),
        1,
        "the open circuit made no further backend call"
    );
    let turns = turns(&server);
    assert!(
        turns
            .iter()
            .any(|(role, text)| *role == TurnRole::Participant && text == "second"),
        "the fast-failed message's inbound is still durable"
    );
}
