//! The deterministic test fakes behind the model seam: [`ScriptedModel`] (programmed responses in
//! order, records what the model saw), [`FlakyModel`] (fault-injection for the resilience paths),
//! and [`stream_response`] (chops a response into word fragments, so every fake streams and every
//! test exercises reassembly). Kept beside the seam rather than under `#[cfg(test)]` because the
//! integration suites and the eval harness drive them too.

use std::{
    collections::VecDeque,
    sync::atomic::{AtomicUsize, Ordering},
};

use async_trait::async_trait;
use parking_lot::Mutex;

use crate::model::{
    Completion, GenerateDelta, GenerateRequest, GenerateResponse, GenerateStream, Message,
    ModelClient, ModelError, ToolChoice, Usage,
};

/// A deterministic fake returning programmed responses in order. Drives agent-loop tests
/// without a real model; `generate` records the request's messages (so a test can assert what the
/// model saw — e.g. that a later turn replayed the live buffer), then pops the next scripted step.
pub struct ScriptedModel {
    steps: Mutex<VecDeque<GenerateResponse>>,
    seen: Mutex<Vec<Vec<Message>>>,
    seen_tool_choice: Mutex<Vec<ToolChoice>>,
}

impl ScriptedModel {
    /// Script the completions a turn will see, each reporting no usage. The common case for tests
    /// that don't exercise the compaction trigger.
    pub fn new(steps: impl IntoIterator<Item = Completion>) -> ScriptedModel {
        ScriptedModel::with_responses(steps.into_iter().map(|completion| GenerateResponse {
            completion,
            usage: Usage::default(),
            reasoning: None,
            finish_reason: None,
        }))
    }

    /// Script completions paired with the `prompt_tokens` each reports, for tests that drive the
    /// compaction trigger deterministically (a step reporting more than the budget forces a
    /// re-segment).
    pub fn with_usage(steps: impl IntoIterator<Item = (Completion, u32)>) -> ScriptedModel {
        ScriptedModel::with_responses(steps.into_iter().map(|(completion, prompt_tokens)| {
            GenerateResponse {
                completion,
                usage: Usage {
                    prompt_tokens: Some(prompt_tokens),
                    ..Usage::default()
                },
                reasoning: None,
                finish_reason: None,
            }
        }))
    }

    /// Script completions paired with the reasoning text and token usage each reports, for tests that
    /// exercise the model-interaction record (the deliberation surface the console captures).
    pub fn with_deliberation(
        steps: impl IntoIterator<Item = (Completion, String, Usage)>,
    ) -> ScriptedModel {
        ScriptedModel::with_responses(steps.into_iter().map(|(completion, reasoning, usage)| {
            GenerateResponse {
                completion,
                usage,
                reasoning: Some(reasoning),
                finish_reason: Some("stop".to_owned()),
            }
        }))
    }

    pub fn with_responses(steps: impl IntoIterator<Item = GenerateResponse>) -> ScriptedModel {
        ScriptedModel {
            steps: Mutex::new(steps.into_iter().collect()),
            seen: Mutex::new(Vec::new()),
            seen_tool_choice: Mutex::new(Vec::new()),
        }
    }

    /// The `messages` of each `generate` call so far, in order — lets a test assert what the model
    /// saw (e.g. that a later turn replayed the prior turns as the prompt suffix).
    pub fn recorded_messages(&self) -> Vec<Vec<Message>> {
        self.seen.lock().clone()
    }

    /// The `tool_choice` of each `generate` call so far, in order — lets a test assert the loop
    /// withdraws the tools (`ToolChoice::None`) on its final step.
    pub fn recorded_tool_choices(&self) -> Vec<ToolChoice> {
        self.seen_tool_choice.lock().clone()
    }
}

#[async_trait]
impl ModelClient for ScriptedModel {
    fn model_id(&self) -> &str {
        "scripted-model"
    }

    /// Streams the next scripted step chopped into word fragments (reasoning first, then a reply's
    /// text) before the terminal — so every scripted test exercises the same fragment-reassembly
    /// path a production run streams through, not a degenerate single-delta stream.
    async fn generate_stream(&self, request: &GenerateRequest) -> GenerateStream {
        self.seen.lock().push(request.messages.clone());
        self.seen_tool_choice.lock().push(request.tool_choice);
        let step = self.steps.lock().pop_front().ok_or(ModelError::Exhausted);
        stream_response(step)
    }
}

/// Chop a response into word fragments ending in its terminal — the streamed shape of a scripted
/// step. Public so any ad-hoc test fake implements `generate_stream` in one line over whatever
/// response it computes: `stream_response(step)`.
pub fn stream_response(step: Result<GenerateResponse, ModelError>) -> GenerateStream {
    let response = match step {
        Ok(response) => response,
        Err(error) => return Box::pin(futures_util::stream::once(async move { Err(error) })),
    };
    let mut deltas = Vec::new();
    if let Some(reasoning) = &response.reasoning {
        deltas.extend(
            reasoning
                .split_inclusive(' ')
                .map(|word| GenerateDelta::Reasoning(word.to_owned())),
        );
    }
    if let Completion::Reply(reply) = &response.completion {
        deltas.extend(
            reply
                .split_inclusive(' ')
                .map(|word| GenerateDelta::Reply(word.to_owned())),
        );
    }
    deltas.push(GenerateDelta::Finished(response));
    Box::pin(futures_util::stream::iter(deltas.into_iter().map(Ok)))
}

/// A fault-injecting fake: fails a programmed number of leading calls (or every call) with a
/// backend error, then serves scripted completions like [`ScriptedModel`]. Drives the
/// transport-resilience paths — retries, the circuit breaker, deferred turns — without a real
/// endpoint, and is distinguishable from [`ScriptedModel`], which never fails with a backend error
/// (its only failure is [`ModelError::Exhausted`], a test-logic error).
pub struct FlakyModel {
    /// How many leading calls fail; `None` means every call fails.
    remaining_faults: Mutex<Option<usize>>,
    /// The failure each faulted call returns, as a rebuildable template (`ModelError` is not `Clone`).
    fault_message: String,
    fault_transient: bool,
    then: ScriptedModel,
    calls: AtomicUsize,
}

impl FlakyModel {
    /// Fail the first `faults` calls with a transient backend error, then serve `steps`.
    pub fn transient_then(
        faults: usize,
        steps: impl IntoIterator<Item = Completion>,
    ) -> FlakyModel {
        FlakyModel {
            remaining_faults: Mutex::new(Some(faults)),
            fault_message: "error sending request (injected transient fault)".to_owned(),
            fault_transient: true,
            then: ScriptedModel::new(steps),
            calls: AtomicUsize::new(0),
        }
    }

    /// Fail every call with a transient backend error — a backend that stays down.
    pub fn always_transient() -> FlakyModel {
        FlakyModel {
            remaining_faults: Mutex::new(None),
            fault_message: "error sending request (injected transient fault)".to_owned(),
            fault_transient: true,
            then: ScriptedModel::new([]),
            calls: AtomicUsize::new(0),
        }
    }

    /// Fail every call with a non-transient backend error — a backend that rejects the request
    /// (schema, auth, a plain 4xx), which must be neither retried nor deferred.
    pub fn always_permanent() -> FlakyModel {
        FlakyModel {
            remaining_faults: Mutex::new(None),
            fault_message: "the backend rejected the request (injected permanent fault)".to_owned(),
            fault_transient: false,
            then: ScriptedModel::new([]),
            calls: AtomicUsize::new(0),
        }
    }

    /// How many `generate` calls reached this model — what a retry or fast-fail assertion counts.
    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    fn fault(&self) -> ModelError {
        ModelError::Backend {
            model: String::new(),
            message: self.fault_message.clone(),
            transient: self.fault_transient,
        }
    }
}

#[async_trait]
impl ModelClient for FlakyModel {
    fn model_id(&self) -> &str {
        "flaky-model"
    }

    async fn generate_stream(&self, request: &GenerateRequest) -> GenerateStream {
        self.calls.fetch_add(1, Ordering::SeqCst);
        // Decide under the guard, then release it before delegating: the scripted delegate's
        // stream opening is an await point, and the synchronous lock must not be held across it.
        let faulted = {
            let mut remaining = self.remaining_faults.lock();
            match remaining.as_mut() {
                None => true,
                Some(0) => false,
                Some(n) => {
                    *n -= 1;
                    true
                }
            }
        };
        if faulted {
            let fault = self.fault();
            return Box::pin(futures_util::stream::once(async move { Err(fault) }));
        }
        self.then.generate_stream(request).await
    }
}
