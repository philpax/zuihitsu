//! The wrapper's retry and circuit-breaker mechanics over the fault-injecting [`FlakyModel`]:
//! transient failures retry then succeed, non-transient failures pass through untouched, and
//! the breaker opens, fast-fails, probes, and closes. Timing-sensitive tests use millisecond
//! windows, so they run in real time without flakiness margins worth worrying about.
use std::sync::Arc;

use crate::{
    config::ResilienceConfig,
    model::{
        Completion, FlakyModel, GenerateRequest, ModelClient, ModelError,
        retry::{CircuitState, RetryingModel, jittered},
    },
};
use std::time::Duration;

/// A policy with instant backoff and a tiny open window, so tests run in milliseconds.
fn tiny(max_attempts: u32, threshold: u32) -> ResilienceConfig {
    ResilienceConfig {
        request_timeout_seconds: 1,
        max_attempts,
        backoff_base_ms: 1,
        backoff_max_ms: 2,
        breaker_failure_threshold: threshold,
        breaker_open_seconds: 0,
    }
}

fn reply() -> Completion {
    Completion::Reply("ok".to_owned())
}

#[tokio::test]
async fn retries_transient_failures_then_succeeds() {
    let flaky = Arc::new(FlakyModel::transient_then(2, [reply()]));
    let model = RetryingModel::new(flaky.clone(), &tiny(3, 10));
    let response = model.generate(&GenerateRequest::default()).await;
    assert!(response.is_ok());
    assert_eq!(flaky.calls(), 3, "two retries after the first failure");
    assert_eq!(model.health().circuit, CircuitState::Closed);
    assert_eq!(model.health().consecutive_failures, 0);
}

#[tokio::test]
async fn a_non_transient_failure_is_not_retried() {
    let flaky = Arc::new(FlakyModel::always_permanent());
    let model = RetryingModel::new(flaky.clone(), &tiny(3, 10));
    let error = model.generate(&GenerateRequest::default()).await;
    assert!(matches!(
        error,
        Err(ModelError::Backend {
            transient: false,
            ..
        })
    ));
    assert_eq!(flaky.calls(), 1, "no retry of a non-transient failure");
    // A non-transient answer proves the backend is reachable — the breaker stays quiet.
    assert_eq!(model.health().circuit, CircuitState::Closed);
    assert_eq!(model.health().consecutive_failures, 0);
}

#[tokio::test]
async fn exhausted_attempts_surface_the_transient_error() {
    let flaky = Arc::new(FlakyModel::always_transient());
    let model = RetryingModel::new(flaky.clone(), &tiny(3, 10));
    let error = model.generate(&GenerateRequest::default()).await;
    assert!(matches!(
        error,
        Err(ModelError::Backend {
            transient: true,
            ..
        })
    ));
    assert_eq!(flaky.calls(), 3, "the attempt bound holds");
    let health = model.health();
    assert_eq!(health.consecutive_failures, 3);
    assert!(health.last_failure.is_some(), "the cause is kept");
}

#[tokio::test]
async fn the_circuit_opens_at_the_threshold_and_fails_fast() {
    // One attempt per call and a threshold of 3: the third failing call opens the circuit.
    let flaky = Arc::new(FlakyModel::always_transient());
    let model = RetryingModel::new(
        flaky.clone(),
        &ResilienceConfig {
            breaker_open_seconds: 3_600, // effectively never half-opens within the test
            ..tiny(1, 3)
        },
    );
    for _ in 0..3 {
        assert!(model.generate(&GenerateRequest::default()).await.is_err());
    }
    assert_eq!(model.health().circuit, CircuitState::Open);
    assert!(model.health().open_remaining_ms.is_some());

    // While open, calls fail fast with the distinct variant and never reach the backend.
    let error = model.generate(&GenerateRequest::default()).await;
    assert!(matches!(error, Err(ModelError::CircuitOpen { .. })));
    assert!(error.unwrap_err().is_unavailable());
    assert_eq!(flaky.calls(), 3, "the fast-fail made no backend call");
}

#[tokio::test]
async fn a_half_open_probe_success_closes_the_circuit() {
    // Two faults open the circuit (threshold 2, one attempt per call); the model then recovers.
    let flaky = Arc::new(FlakyModel::transient_then(2, [reply(), reply()]));
    let model = RetryingModel::new(flaky.clone(), &tiny(1, 2));
    for _ in 0..2 {
        assert!(model.generate(&GenerateRequest::default()).await.is_err());
    }
    assert_eq!(model.health().circuit, CircuitState::Open);

    // The zero-second window has lapsed: the next call is the probe, it succeeds, and the
    // circuit closes — the call after it flows normally.
    assert!(model.generate(&GenerateRequest::default()).await.is_ok());
    assert_eq!(model.health().circuit, CircuitState::Closed);
    assert_eq!(model.health().consecutive_failures, 0);
    assert!(model.generate(&GenerateRequest::default()).await.is_ok());
    assert_eq!(flaky.calls(), 4);
}

#[tokio::test]
async fn a_half_open_probe_failure_reopens_the_circuit() {
    // A zero-second open window: the call after the circuit opens is immediately the probe.
    let flaky = Arc::new(FlakyModel::always_transient());
    let model = RetryingModel::new(flaky.clone(), &tiny(1, 2));
    for _ in 0..2 {
        assert!(model.generate(&GenerateRequest::default()).await.is_err());
    }
    assert_eq!(model.health().circuit, CircuitState::Open);

    let calls_before_probe = flaky.calls();
    let error = model.generate(&GenerateRequest::default()).await;
    assert!(
        matches!(
            error,
            Err(ModelError::Backend {
                transient: true,
                ..
            })
        ),
        "the probe's own failure surfaces as the backend error"
    );
    assert_eq!(flaky.calls(), calls_before_probe + 1, "exactly one probe");
    assert_eq!(model.health().circuit, CircuitState::Open);
}

#[tokio::test]
async fn retry_and_fast_fail_metrics_are_observed() {
    use crate::metrics::{
        LATENCY_BUCKETS, MODEL_CIRCUIT_FAST_FAILS_TOTAL, MODEL_CIRCUIT_STATE, MODEL_RETRIES_TOTAL,
        describe,
    };
    let recorder = metrics_exporter_prometheus::PrometheusBuilder::new()
        .set_buckets(LATENCY_BUCKETS)
        .unwrap()
        .build_recorder();
    let handle = recorder.handle();
    let _guard = metrics::set_default_local_recorder(&recorder);
    describe();

    let flaky = Arc::new(FlakyModel::always_transient());
    let model = RetryingModel::new(
        flaky,
        &ResilienceConfig {
            breaker_open_seconds: 3_600,
            ..tiny(3, 3)
        },
    );
    // Three attempts → two retries; the third failure opens the circuit; one more call fast-fails.
    assert!(model.generate(&GenerateRequest::default()).await.is_err());
    assert!(model.generate(&GenerateRequest::default()).await.is_err());

    let text = handle.render();
    assert!(
        text.contains(&format!("{MODEL_RETRIES_TOTAL} 2\n")),
        "two retries observed: {text}"
    );
    assert!(
        text.contains(&format!("{MODEL_CIRCUIT_FAST_FAILS_TOTAL} 1\n")),
        "one fast-fail observed: {text}"
    );
    assert!(
        text.contains(&format!("{MODEL_CIRCUIT_STATE} 2\n")),
        "the state gauge reads open (2): {text}"
    );
}

#[test]
fn jitter_stays_within_the_delay() {
    for _ in 0..100 {
        let jittered = jittered(Duration::from_millis(100));
        assert!(jittered >= Duration::from_millis(50));
        assert!(jittered <= Duration::from_millis(100));
    }
    // A sub-millisecond delay passes through unjittered rather than dividing to zero.
    assert_eq!(jittered(Duration::from_millis(1)), Duration::from_millis(1));
}
