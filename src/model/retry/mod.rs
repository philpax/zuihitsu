//! Transport resilience for the model seam: a [`ModelClient`] decorator that retries transient
//! backend failures with exponential backoff and jitter, and a circuit breaker that fails fast
//! while the backend stays down.
//!
//! Retries the agent never saw are infra-transparent (spec §Transport resilience): they emit nothing to
//! the event log — tracing and metrics only — so replay never depends on the retry policy. The
//! policy itself is operational config ([`ResilienceConfig`], `[model.resilience]` in
//! `config.toml`), not behavioral `Settings`. The serving host wraps the real OpenAI client in
//! this at construction; a directly constructed model stays raw unless its caller opts in.

use std::{
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use parking_lot::Mutex;

use crate::{
    config::ResilienceConfig,
    metrics::{observe_model_circuit_fast_fail, observe_model_retry, set_model_circuit_state},
};

use futures_util::StreamExt;

use crate::model::{GenerateDelta, GenerateRequest, GenerateStream, ModelClient, ModelError};

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
    /// Shared with the streams `generate_stream` yields, which outlive the call that made them and
    /// must still record their terminal outcome against the breaker.
    breaker: Arc<BreakerShared>,
}

/// The breaker state plus what its accounting needs to log and threshold — the part of the model a
/// detached stream can carry.
struct BreakerShared {
    model: String,
    failure_threshold: u32,
    breaker: Mutex<Breaker>,
}

impl RetryingModel {
    pub fn new(inner: Arc<dyn ModelClient>, config: &ResilienceConfig) -> RetryingModel {
        set_model_circuit_state(CircuitState::Closed);
        RetryingModel {
            breaker: Arc::new(BreakerShared {
                model: inner.model_id().to_owned(),
                failure_threshold: config.breaker_failure_threshold.max(1),
                breaker: Mutex::new(Breaker {
                    state: State::Closed,
                    consecutive_failures: 0,
                    last_failure: None,
                }),
            }),
            inner,
            config: config.clone(),
        }
    }

    /// The transport's health for the operator surface (`GET /control/health`): the circuit state,
    /// the consecutive-failure count, and the last failure's cause.
    pub fn health(&self) -> BackendHealth {
        let breaker = self.breaker.breaker.lock();
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
    /// half-open probe (this call is the probe — the returned [`ProbeGuard`] re-opens the circuit
    /// if the probe is dropped unresolved), and fail fast otherwise — while open, and while a probe
    /// is already in flight.
    fn admit(&self) -> Result<Option<ProbeGuard>, ModelError> {
        let mut breaker = self.breaker.breaker.lock();
        match breaker.state {
            State::Closed => Ok(None),
            State::Open { since }
                if since.elapsed() >= Duration::from_secs(self.config.breaker_open_seconds) =>
            {
                breaker.set_state(State::HalfOpen);
                tracing::info!(
                    model = self.inner.model_id(),
                    "the circuit's open window lapsed; sending a half-open probe"
                );
                Ok(Some(ProbeGuard {
                    breaker: self.breaker.clone(),
                    armed: true,
                }))
            }
            State::Open { .. } | State::HalfOpen => {
                observe_model_circuit_fast_fail();
                Err(ModelError::CircuitOpen {
                    model: self.inner.model_id().to_owned(),
                })
            }
        }
    }
}

/// Armed while the half-open probe is unresolved. Streaming widens the probe's life from one HTTP
/// round trip to a whole generation, so a probe stream dropped mid-flight — a cancelled turn, a
/// hung caller — would otherwise leave the breaker in `HalfOpen`, fast-failing every future call
/// forever. Dropping the guard unresolved re-opens the circuit instead, so the next lapsed window
/// sends a fresh probe.
struct ProbeGuard {
    breaker: Arc<BreakerShared>,
    armed: bool,
}

impl ProbeGuard {
    /// The probe reached a terminal (either recorder ran); the guard stands down.
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for ProbeGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let mut breaker = self.breaker.breaker.lock();
        if breaker.state == State::HalfOpen {
            tracing::warn!(
                model = self.breaker.model,
                "the half-open probe was dropped unresolved; re-opening the circuit"
            );
            breaker.set_state(State::Open {
                since: Instant::now(),
            });
        }
    }
}

impl BreakerShared {
    /// Record that the backend answered. A success — and equally a *non-transient* error, which
    /// still proves the backend is reachable and responding — closes the circuit and resets the
    /// failure count. Without the non-transient case a half-open probe answered with, say, a 400
    /// would leave the breaker stuck in `HalfOpen`, fast-failing forever.
    fn record_reachable(&self) {
        let mut breaker = self.breaker.lock();
        breaker.consecutive_failures = 0;
        if breaker.state != State::Closed {
            tracing::info!(
                model = self.model,
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
                    model = self.model,
                    %error,
                    "the half-open probe failed; re-opening the circuit"
                );
                breaker.set_state(State::Open {
                    since: Instant::now(),
                });
                true
            }
            State::Closed if breaker.consecutive_failures >= self.failure_threshold => {
                tracing::warn!(
                    model = self.model,
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

    /// Streaming with restart-on-transient-failure at any point. An attempt that fails
    /// transiently — before or after fragments were delivered — is discarded whole: the stream
    /// yields a [`GenerateDelta::Restarted`] marker (so a consumer voids what it accumulated and,
    /// in the turn loop, records the discarded partial as a `ModelCallAborted`), backs off, and
    /// re-drives the request from scratch. The terminal is therefore always a complete,
    /// single-attempt response. Non-transient failures propagate immediately (the backend
    /// answered; retrying will not change a 400), and every terminal outcome feeds the breaker.
    /// The unary `generate` is the trait's drain of this same stream, so retried unary calls and
    /// retried streams are one code path.
    async fn generate_stream(&self, request: &GenerateRequest) -> GenerateStream {
        let mut probe = match self.admit() {
            Ok(probe) => probe,
            Err(error) => {
                return Box::pin(futures_util::stream::once(async move { Err(error) }));
            }
        };
        let inner = self.inner.clone();
        let breaker = self.breaker.clone();
        let request = request.clone();
        let max_attempts = self.config.max_attempts.max(1);
        let backoff_max = Duration::from_millis(self.config.backoff_max_ms);
        let backoff_base = Duration::from_millis(self.config.backoff_base_ms).min(backoff_max);
        Box::pin(async_stream::stream! {
            let mut backoff = backoff_base;
            let mut attempt = 1u32;
            'attempts: loop {
                let mut stream = inner.generate_stream(&request).await;
                loop {
                    match stream.next().await {
                        Some(Ok(delta)) => {
                            let finished = matches!(delta, GenerateDelta::Finished(_));
                            if finished {
                                // Recorded before the yield: the consumer may drop the stream the
                                // moment it holds the terminal, and a suspended generator never
                                // resumes to run anything after that yield.
                                breaker.record_reachable();
                                if let Some(probe) = &mut probe {
                                    probe.disarm();
                                }
                            }
                            yield Ok(delta);
                            if finished {
                                return;
                            }
                        }
                        Some(Err(error)) if error.is_transient() => {
                            let opened = breaker.record_transient_failure(&error);
                            if let Some(probe) = &mut probe {
                                probe.disarm();
                            }
                            if opened || attempt >= max_attempts {
                                // Exhausted, or the breaker just declared the backend down —
                                // surface the transient error itself (the caller's deferral reads
                                // `is_unavailable`).
                                yield Err(error);
                                return;
                            }
                            tracing::warn!(
                                model = inner.model_id(),
                                attempt,
                                max_attempts,
                                backoff_ms = backoff.as_millis() as u64,
                                %error,
                                "transient model failure; discarding the attempt and re-driving"
                            );
                            observe_model_retry();
                            yield Ok(GenerateDelta::Restarted {
                                attempt,
                                cause: error.to_string(),
                            });
                            tokio::time::sleep(jittered(backoff)).await;
                            backoff = (backoff * 2).min(backoff_max);
                            attempt += 1;
                            continue 'attempts;
                        }
                        Some(Err(error)) => {
                            // A non-transient failure still proves the backend answered (see
                            // `BreakerShared::record_reachable`); it is never retried. Recorded
                            // before the yield for the same drop-after-terminal reason as above.
                            breaker.record_reachable();
                            if let Some(probe) = &mut probe {
                                probe.disarm();
                            }
                            yield Err(error);
                            return;
                        }
                        None => {
                            // Ended without a terminal: an inner-contract violation, surfaced
                            // rather than silently inventing a response. Deliberately bypasses the
                            // breaker accounting and leaves any probe armed: a violated contract
                            // says nothing about backend health, and re-opening the circuit on an
                            // unresolved probe is the safe default.
                            yield Err(ModelError::Backend {
                                model: inner.model_id().to_owned(),
                                message: "the stream ended without a terminal response".to_owned(),
                                transient: false,
                            });
                            return;
                        }
                    }
                }
            }
        })
    }
}

// The wire-contract types `CircuitState` and `BackendHealth` are defined in
// `zuihitsu-frontend-types` and re-exported at the crate root.
pub use zuihitsu_frontend_types::{BackendHealth, CircuitState};

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
mod tests;
