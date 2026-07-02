//! Transport resilience for the model seam: a [`ModelClient`] decorator that retries transient
//! backend failures with exponential backoff and jitter, and a circuit breaker that fails fast
//! while the backend stays down.
//!
//! Retries the agent never saw are infra-transparent (spec §Event sourcing): they emit nothing to
//! the event log — tracing and metrics only — so replay never depends on the retry policy. The
//! policy itself is operational config ([`ResilienceConfig`], `[model.resilience]` in
//! `config.toml`), not behavioral `Settings`. The serving host wraps the real OpenAI client in
//! this at construction; the eval harness and tests keep their raw models unless they opt in.

use std::{
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use parking_lot::Mutex;
use serde::Serialize;

use crate::{
    config::ResilienceConfig,
    metrics::{observe_model_circuit_fast_fail, observe_model_retry, set_model_circuit_state},
};

use super::{GenerateRequest, GenerateResponse, ModelClient, ModelError};

/// A [`ModelClient`] that retries transient failures (bounded attempts, exponential backoff with
/// jitter) and holds the circuit breaker: after `breaker_failure_threshold` consecutive transient
/// failures the circuit opens, and model-needing calls fail fast with
/// [`ModelError::CircuitOpen`] — no backend call — until the open window lapses and one half-open
/// probe request decides whether to close it. Breaker state is in-memory on the wrapper
/// (operational, never logged). One instance is shared by every caller of the model — the turn
/// loop and the background workers — so "the backend is down" is discovered once, not per caller.
pub struct RetryingModel {
    inner: Arc<dyn ModelClient>,
    config: ResilienceConfig,
    breaker: Mutex<Breaker>,
}

impl RetryingModel {
    pub fn new(inner: Arc<dyn ModelClient>, config: &ResilienceConfig) -> RetryingModel {
        set_model_circuit_state(CircuitState::Closed);
        RetryingModel {
            inner,
            config: config.clone(),
            breaker: Mutex::new(Breaker {
                state: State::Closed,
                consecutive_failures: 0,
                last_failure: None,
            }),
        }
    }

    /// The transport's health for the operator surface (`GET /control/health`): the circuit state,
    /// the consecutive-failure count, and the last failure's cause.
    pub fn health(&self) -> BackendHealth {
        let breaker = self.breaker.lock();
        let open_for = Duration::from_secs(self.config.breaker_open_seconds);
        BackendHealth {
            circuit: breaker.state.view(),
            consecutive_failures: breaker.consecutive_failures,
            last_failure: breaker.last_failure.clone(),
            open_remaining_ms: match breaker.state {
                State::Open { since } => Some(
                    open_for
                        .saturating_sub(since.elapsed())
                        .as_millis()
                        .min(u128::from(u64::MAX)) as u64,
                ),
                State::Closed | State::HalfOpen => None,
            },
        }
    }

    /// Gate one call on the circuit: pass when closed, convert a lapsed open window into the
    /// half-open probe (this call is the probe), and fail fast otherwise — while open, and while a
    /// probe is already in flight.
    fn admit(&self) -> Result<(), ModelError> {
        let mut breaker = self.breaker.lock();
        match breaker.state {
            State::Closed => Ok(()),
            State::Open { since }
                if since.elapsed() >= Duration::from_secs(self.config.breaker_open_seconds) =>
            {
                breaker.set_state(State::HalfOpen);
                tracing::info!(
                    model = self.inner.model_id(),
                    "the circuit's open window lapsed; sending a half-open probe"
                );
                Ok(())
            }
            State::Open { .. } | State::HalfOpen => {
                observe_model_circuit_fast_fail();
                Err(ModelError::CircuitOpen {
                    model: self.inner.model_id().to_owned(),
                })
            }
        }
    }

    /// Record that the backend answered. A success — and equally a *non-transient* error, which
    /// still proves the backend is reachable and responding — closes the circuit and resets the
    /// failure count. Without the non-transient case a half-open probe answered with, say, a 400
    /// would leave the breaker stuck in `HalfOpen`, fast-failing forever.
    fn record_reachable(&self) {
        let mut breaker = self.breaker.lock();
        breaker.consecutive_failures = 0;
        if breaker.state != State::Closed {
            tracing::info!(
                model = self.inner.model_id(),
                "the backend answered; closing the circuit"
            );
        }
        breaker.set_state(State::Closed);
    }

    /// Record one transient failure: bump the consecutive count, keep the cause for the health
    /// surface, and open the circuit when the threshold is crossed (or re-open it when a half-open
    /// probe failed). Returns whether the circuit is now open, so the retry loop stops rather than
    /// hammering a backend the breaker just declared down.
    fn record_transient_failure(&self, error: &ModelError) -> bool {
        let mut breaker = self.breaker.lock();
        breaker.consecutive_failures = breaker.consecutive_failures.saturating_add(1);
        breaker.last_failure = Some(error.to_string());
        match breaker.state {
            State::HalfOpen => {
                tracing::warn!(
                    model = self.inner.model_id(),
                    %error,
                    "the half-open probe failed; re-opening the circuit"
                );
                breaker.set_state(State::Open {
                    since: Instant::now(),
                });
                true
            }
            State::Closed
                if breaker.consecutive_failures >= self.config.breaker_failure_threshold.max(1) =>
            {
                tracing::warn!(
                    model = self.inner.model_id(),
                    consecutive_failures = breaker.consecutive_failures,
                    %error,
                    "consecutive transient failures crossed the threshold; opening the circuit"
                );
                breaker.set_state(State::Open {
                    since: Instant::now(),
                });
                true
            }
            State::Closed => false,
            // Another concurrent call already opened it; refresh the window so the probe waits a
            // full quiet period from the most recent failure.
            State::Open { .. } => {
                breaker.set_state(State::Open {
                    since: Instant::now(),
                });
                true
            }
        }
    }
}

#[async_trait]
impl ModelClient for RetryingModel {
    fn model_id(&self) -> &str {
        self.inner.model_id()
    }

    async fn generate(&self, request: &GenerateRequest) -> Result<GenerateResponse, ModelError> {
        self.admit()?;
        let max_attempts = self.config.max_attempts.max(1);
        let backoff_max = Duration::from_millis(self.config.backoff_max_ms);
        let mut backoff = Duration::from_millis(self.config.backoff_base_ms).min(backoff_max);
        let mut attempt = 1u32;
        loop {
            match self.inner.generate(request).await {
                Ok(response) => {
                    self.record_reachable();
                    return Ok(response);
                }
                Err(error) if error.is_transient() => {
                    let opened = self.record_transient_failure(&error);
                    if opened || attempt >= max_attempts {
                        // Exhausted, or the breaker just declared the backend down — surface the
                        // transient error itself (the caller's deferral reads `is_unavailable`).
                        return Err(error);
                    }
                    tracing::warn!(
                        model = self.inner.model_id(),
                        attempt,
                        max_attempts,
                        backoff_ms = backoff.as_millis() as u64,
                        %error,
                        "transient model failure; backing off and retrying"
                    );
                    observe_model_retry();
                    tokio::time::sleep(jittered(backoff)).await;
                    backoff = (backoff * 2).min(backoff_max);
                    attempt += 1;
                }
                Err(error) => {
                    // A non-transient failure still proves the backend answered (see
                    // `record_reachable`); it is never retried.
                    self.record_reachable();
                    return Err(error);
                }
            }
        }
    }
}

/// The circuit's observable state, for the operator health surface and the state gauge.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum CircuitState {
    Closed,
    HalfOpen,
    Open,
}

/// The model transport's health, as the operator surface reports it: the circuit state, the
/// consecutive transient-failure count, the last failure's cause (kept across recovery, so an
/// operator can still read what went wrong), and — while open — how long until the half-open probe.
#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct BackendHealth {
    pub circuit: CircuitState,
    pub consecutive_failures: u32,
    pub last_failure: Option<String>,
    pub open_remaining_ms: Option<u64>,
}

/// The breaker's in-memory state. Wholly operational: never logged, reset on restart (a restarted
/// server probes the backend afresh, which is the desired posture).
struct Breaker {
    state: State,
    consecutive_failures: u32,
    last_failure: Option<String>,
}

impl Breaker {
    /// Transition the state, keeping the Prometheus gauge in lockstep.
    fn set_state(&mut self, state: State) {
        set_model_circuit_state(state.view());
        self.state = state;
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Closed,
    /// Failing fast since `since`; after the configured window one probe is admitted.
    Open {
        since: Instant,
    },
    /// The probe is in flight; other calls still fail fast until it resolves.
    HalfOpen,
}

impl State {
    fn view(self) -> CircuitState {
        match self {
            State::Closed => CircuitState::Closed,
            State::Open { .. } => CircuitState::Open,
            State::HalfOpen => CircuitState::HalfOpen,
        }
    }
}

/// Jitter a backoff delay into `[delay/2, delay]`, seeded from the wall clock's sub-second nanos —
/// enough spread to de-synchronize concurrent retry loops without a rand dependency.
fn jittered(delay: Duration) -> Duration {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| u64::from(since.subsec_nanos()))
        .unwrap_or(0);
    let half = (delay.as_millis() as u64) / 2;
    if half == 0 {
        return delay;
    }
    Duration::from_millis(half + nanos % (half + 1))
}

#[cfg(test)]
mod tests {
    //! The wrapper's retry and circuit-breaker mechanics over the fault-injecting [`FlakyModel`]:
    //! transient failures retry then succeed, non-transient failures pass through untouched, and
    //! the breaker opens, fast-fails, probes, and closes. Timing-sensitive tests use millisecond
    //! windows, so they run in real time without flakiness margins worth worrying about.
    use std::sync::Arc;

    use super::{CircuitState, RetryingModel, jittered};
    use crate::{
        config::ResilienceConfig,
        model::{Completion, FlakyModel, GenerateRequest, ModelClient, ModelError},
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
            LATENCY_BUCKETS, MODEL_CIRCUIT_FAST_FAILS_TOTAL, MODEL_CIRCUIT_STATE,
            MODEL_RETRIES_TOTAL, describe,
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
}
