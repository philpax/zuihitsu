//! Endpoint resilience for the eval lane: thin retrying wrappers over the model and embedder seams.
//!
//! A soak drives hundreds of serialized inference calls over a long window, so a brief endpoint
//! outage — a scheduled host rebuild, a serving-layer restart — would otherwise abort whichever runs
//! coincide with it and count them as quality failures, conflating infrastructure with the model (spec
//! §Validation → the rate is a quality signal). These wrappers retry a `Backend` failure with
//! exponential backoff up to a five-minute budget, so a transient outage costs latency rather than a
//! lost run. `Exhausted` (a scripted fake out of responses) is a test-logic error and is never retried.
//!
//! This lives in the eval crate, not behind the model seam itself, deliberately: a live agent turn has
//! a human waiting and must fail fast, whereas an unattended soak can afford to wait out a rebuild.

use std::{
    future::Future,
    sync::Arc,
    time::{Duration, Instant},
};

use async_trait::async_trait;
use futures_util::StreamExt;
use zuihitsu::{
    Embedder, Embedding, GenerateDelta, GenerateRequest, GenerateStream, ModelClient, ModelError,
};

/// The backoff schedule: the first delay, the per-retry ceiling, and the total time to keep trying
/// before giving up. The production values ride out a host rebuild; tests pass tiny ones.
#[derive(Clone, Copy)]
pub struct Backoff {
    pub base: Duration,
    pub max: Duration,
    pub budget: Duration,
}

impl Default for Backoff {
    fn default() -> Backoff {
        Backoff {
            base: Duration::from_secs(1),
            max: Duration::from_secs(60),
            budget: Duration::from_secs(300),
        }
    }
}

/// A [`ModelClient`] that retries transient backend failures (see module docs).
pub struct RetryingModel {
    inner: Arc<dyn ModelClient>,
    backoff: Backoff,
}

impl RetryingModel {
    pub fn new(inner: Arc<dyn ModelClient>) -> RetryingModel {
        RetryingModel {
            inner,
            backoff: Backoff::default(),
        }
    }
}

#[async_trait]
impl ModelClient for RetryingModel {
    fn model_id(&self) -> &str {
        self.inner.model_id()
    }

    /// Streaming with restart-on-failure under the backoff budget: an attempt that fails at any
    /// point — even mid-stream — is discarded whole, a [`GenerateDelta::Restarted`] marker voids
    /// what the consumer accumulated (the turn loop records it as a `ModelCallAborted`), and the
    /// request re-drives after the delay. `Exhausted` is a test-logic error, never retried. The
    /// unary `generate` is the trait's drain of this same stream.
    async fn generate_stream(&self, request: &GenerateRequest) -> GenerateStream {
        let inner = self.inner.clone();
        let cfg = self.backoff;
        let request = request.clone();
        Box::pin(async_stream::stream! {
            let start = Instant::now();
            let mut delay = cfg.base;
            let mut attempt = 1u32;
            'attempts: loop {
                let mut stream = inner.generate_stream(&request).await;
                loop {
                    match stream.next().await {
                        Some(Ok(delta)) => {
                            let finished = matches!(delta, GenerateDelta::Finished(_));
                            yield Ok(delta);
                            if finished {
                                return;
                            }
                        }
                        Some(Err(ModelError::Exhausted)) => {
                            yield Err(ModelError::Exhausted);
                            return;
                        }
                        Some(Err(error)) => {
                            if start.elapsed() + delay > cfg.budget {
                                tracing::warn!(call = "generate_stream", %error, "model endpoint still failing after the backoff budget; giving up");
                                yield Err(error);
                                return;
                            }
                            tracing::warn!(
                                call = "generate_stream",
                                %error,
                                backoff_secs = delay.as_secs_f64(),
                                elapsed_secs = start.elapsed().as_secs(),
                                "model endpoint failed; discarding the attempt and retrying"
                            );
                            yield Ok(GenerateDelta::Restarted {
                                attempt,
                                cause: error.to_string(),
                            });
                            tokio::time::sleep(delay).await;
                            delay = (delay * 2).min(cfg.max);
                            attempt += 1;
                            continue 'attempts;
                        }
                        None => {
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

/// An [`Embedder`] that retries transient backend failures (see module docs).
pub struct RetryingEmbedder {
    inner: Arc<dyn Embedder>,
    backoff: Backoff,
}

impl RetryingEmbedder {
    pub fn new(inner: Arc<dyn Embedder>) -> RetryingEmbedder {
        RetryingEmbedder {
            inner,
            backoff: Backoff::default(),
        }
    }
}

#[async_trait]
impl Embedder for RetryingEmbedder {
    fn dimensions(&self) -> usize {
        self.inner.dimensions()
    }

    fn model_id(&self) -> &str {
        self.inner.model_id()
    }

    async fn embed(&self, inputs: &[String]) -> Result<Vec<Embedding>, ModelError> {
        with_backoff(self.backoff, "embed", || self.inner.embed(inputs)).await
    }
}

/// Run `op`, retrying a `Backend` error with exponential backoff until it succeeds or the next delay
/// would exceed the budget. Success and `Exhausted` short-circuit. `label` names the call in the log.
async fn with_backoff<T, F, Fut>(cfg: Backoff, label: &str, op: F) -> Result<T, ModelError>
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<T, ModelError>>,
{
    let start = Instant::now();
    let mut delay = cfg.base;
    loop {
        match op().await {
            Ok(value) => return Ok(value),
            Err(ModelError::Exhausted) => return Err(ModelError::Exhausted),
            Err(error) => {
                if start.elapsed() + delay > cfg.budget {
                    tracing::warn!(call = label, %error, "model endpoint still failing after the backoff budget; giving up");
                    return Err(error);
                }
                tracing::warn!(
                    call = label,
                    %error,
                    backoff_secs = delay.as_secs_f64(),
                    elapsed_secs = start.elapsed().as_secs(),
                    "model endpoint failed; backing off and retrying"
                );
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(cfg.max);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use zuihitsu::{Completion, GenerateResponse, Usage};

    fn reply() -> GenerateResponse {
        GenerateResponse {
            completion: Completion::Reply("ok".to_owned()),
            usage: Usage::default(),
            reasoning: None,
            finish_reason: None,
        }
    }

    /// A model that fails `fail_times` with a `Backend` error, then succeeds.
    struct FlakyModel {
        calls: AtomicUsize,
        fail_times: usize,
    }

    #[async_trait]
    impl ModelClient for FlakyModel {
        fn model_id(&self) -> &str {
            "flaky"
        }
        async fn generate_stream(&self, _request: &GenerateRequest) -> GenerateStream {
            let n = self.calls.fetch_add(1, Ordering::SeqCst);
            let step = if n < self.fail_times {
                Err(ModelError::Backend {
                    model: String::new(),
                    message: "error sending request".to_owned(),
                    transient: true,
                })
            } else {
                Ok(reply())
            };
            zuihitsu::stream_response(step)
        }
    }

    fn tiny() -> Backoff {
        Backoff {
            base: Duration::from_millis(1),
            max: Duration::from_millis(4),
            budget: Duration::from_millis(50),
        }
    }

    #[tokio::test]
    async fn retries_then_succeeds() {
        let attempts = AtomicUsize::new(0);
        let result = with_backoff(tiny(), "test", || {
            let n = attempts.fetch_add(1, Ordering::SeqCst);
            async move {
                if n < 2 {
                    Err::<(), _>(ModelError::Backend {
                        model: String::new(),
                        message: "transient".to_owned(),
                        transient: true,
                    })
                } else {
                    Ok(())
                }
            }
        })
        .await;
        assert!(result.is_ok());
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn gives_up_after_budget() {
        let attempts = AtomicUsize::new(0);
        let result: Result<(), _> = with_backoff(tiny(), "test", || {
            attempts.fetch_add(1, Ordering::SeqCst);
            async {
                Err(ModelError::Backend {
                    model: String::new(),
                    message: "always down".to_owned(),
                    transient: true,
                })
            }
        })
        .await;
        assert!(matches!(result, Err(ModelError::Backend { .. })));
        assert!(attempts.load(Ordering::SeqCst) >= 2);
    }

    #[tokio::test]
    async fn does_not_retry_exhausted() {
        let attempts = AtomicUsize::new(0);
        let result: Result<(), _> = with_backoff(tiny(), "test", || {
            attempts.fetch_add(1, Ordering::SeqCst);
            async { Err(ModelError::Exhausted) }
        })
        .await;
        assert!(matches!(result, Err(ModelError::Exhausted)));
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn model_wrapper_rides_out_a_blip() {
        let flaky = Arc::new(FlakyModel {
            calls: AtomicUsize::new(0),
            fail_times: 2,
        });
        let model = RetryingModel {
            inner: flaky,
            backoff: tiny(),
        };
        let response = model.generate(&GenerateRequest::default()).await;
        assert!(response.is_ok());
    }
}
