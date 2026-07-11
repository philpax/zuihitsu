//! The model-client seam: structured text generation. The real client (the local model over the
//! OpenAI-compatible endpoint) is wired in Stage 5; tests use a scripted fake that returns
//! predetermined steps, so an agent-level test is deterministic and needs no GPU (spec
//! §Testability). The request/response shape is deliberately small here and grows with the agent
//! loop and tool protocol in Stage 4.
//!
//! This root holds the model-client seam itself; the embedder seam lives in [`embed`], the
//! log-to-vector indexer in [`index`], the OpenAI-compatible backends for both in [`openai`], and
//! the transport-resilience wrapper (retries plus the circuit breaker) in [`retry`].

pub mod embed;
pub mod index;
pub mod openai;
pub mod priority;
pub mod retry;

pub use priority::ModelArbiter;

use std::{
    collections::VecDeque,
    sync::atomic::{AtomicUsize, Ordering},
};

use async_trait::async_trait;
use parking_lot::Mutex;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

// The model-interaction wire types live in zuihitsu-core (so the event log and the console can
// share them in wasm) and are re-exported here, keeping them reachable at `crate::model::*`.
pub use zuihitsu_core::model::{Completion, Message, Role, ToolCall, ToolChoice, ToolSpec, Usage};

/// A JSON-schema constraint on the whole response body — OpenAI `response_format: { type:
/// "json_schema" }`. For a single structured extraction this is preferable to a forced tool call: some
/// serving layers grammar-constrain the response-format path but leave forced-tool-call *arguments*
/// unconstrained, so a weaker tool-caller free-forms the shape (the Gemma 4 case). The schema is
/// `strict`, and the model's reply carries the JSON (possibly inside a fenced block).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResponseSchema {
    pub name: String,
    pub schema: serde_json::Value,
}

/// What the model is asked to produce a step for.
///
/// The serialized shape (field names and order) is load-bearing beyond the wire: the recorder's
/// `request_digest` hashes `serde_json::to_vec` of this struct, and the console's digest verifier
/// (`RequestDigestView` in `crates/console-wasm`) mirrors it to reproduce that hash from the
/// recorded deltas. A field change here must be mirrored there, or every call reads as a digest
/// mismatch in the console.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct GenerateRequest {
    pub system: String,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolSpec>,
    pub tool_choice: ToolChoice,
    /// Constrain the whole response to a JSON schema instead of offering tools — used for a single
    /// structured extraction (e.g. `synthesize`) where the response-format path is grammar-constrained
    /// even when forced-tool-call arguments are not. Mutually exclusive with `tools` in practice.
    pub response_format: Option<ResponseSchema>,
    /// Per-request override of the serving layer's reasoning mode: `None` uses the configured
    /// default; `Some(false)` forces it off — used for structured extractions (e.g. description
    /// regeneration), where reasoning adds nothing and makes a forced tool call unreliable.
    pub thinking: Option<bool>,
}

impl GenerateRequest {
    /// A single schema-constrained structured-output call: one user message, the whole reply
    /// constrained to `T`'s JSON schema via `response_format`, reasoning off. This is the reliable way
    /// to extract one fixed structured object — the response-format path is grammar-constrained on
    /// serving layers where a forced tool call's *arguments* are not (the Gemma 4 case), so a weak
    /// tool-caller can free-form a schema-wrong shape through a tool but not through this. Read the
    /// reply with [`parse_structured`] (strict) or [`extract_json_object`] (to salvage fields).
    ///
    /// The schema is also rendered into the system prompt. The `response_format` grammar constrains the
    /// reply's *structure* token by token, but a serving layer's chat template does not necessarily show
    /// the model the schema (Gemma's does not), so without this the model is forced into a shape it
    /// cannot see — it satisfies the grammar but mis-assigns values (flattening a nested field, naming
    /// the wrong enum variant). Showing the schema turns the constraint from a straitjacket into an
    /// instruction the model is decoding toward.
    pub fn structured<T: JsonSchema>(
        system: impl Into<String>,
        user: impl Into<String>,
        schema_name: impl Into<String>,
    ) -> GenerateRequest {
        let schema = schema_of::<T>();
        let schema_name = schema_name.into();
        let system = format!(
            "{}\n\nRespond with a single JSON object conforming to this JSON Schema \
             (the `{schema_name}` schema):\n```json\n{}\n```",
            system.into(),
            serde_json::to_string_pretty(&schema).unwrap_or_else(|_| schema.to_string()),
        );
        GenerateRequest {
            system,
            messages: vec![Message::user(user.into())],
            tools: Vec::new(),
            tool_choice: ToolChoice::Auto,
            response_format: Some(ResponseSchema {
                name: schema_name,
                schema,
            }),
            thinking: Some(false),
        }
    }
}

/// The JSON-Schema for a type — the single source of truth shared between the schema sent to the model
/// and the parser that reads its reply.
pub fn schema_of<T: JsonSchema>() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(T)).unwrap_or_default()
}

/// Extract the JSON object from a structured-output reply: the body may arrive bare or inside a fenced
/// block (some chat templates emit an optional thought, then a ```json … ``` block), so take the span
/// from the first `{` to the last `}`. `None` if there is no brace pair.
pub fn extract_json_object(content: &str) -> Option<&str> {
    let start = content.find('{')?;
    let end = content.rfind('}')?;
    (end >= start).then(|| &content[start..=end])
}

/// Parse a structured-output reply into `T`: take the JSON object (see [`extract_json_object`]) and
/// deserialize it strictly. `None` unless the reply is a `Reply` carrying a parseable `T`.
pub fn parse_structured<T: DeserializeOwned>(completion: &Completion) -> Option<T> {
    let Completion::Reply(content) = completion else {
        return None;
    };
    serde_json::from_str(extract_json_object(content)?).ok()
}

/// One generation step's result: the [`Completion`] the loop acts on, plus the [`Usage`] the
/// compaction trigger reads, and the deliberation surface the model-interaction record captures —
/// `reasoning` (the serving layer's `reasoning_content`, when present) and the `finish_reason`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenerateResponse {
    pub completion: Completion,
    pub usage: Usage,
    pub reasoning: Option<String>,
    pub finish_reason: Option<String>,
}

/// The inference interface. The agent server holds one of these; tests substitute a fake.
#[async_trait]
pub trait ModelClient: Send + Sync {
    /// The id of the model behind this client, recorded as `produced_by` provenance on the events
    /// its inference produces (spec §Storage → provenance on inference).
    fn model_id(&self) -> &str;
    async fn generate(&self, request: &GenerateRequest) -> Result<GenerateResponse, ModelError>;
}

/// A model-inference failure.
#[derive(Debug)]
pub enum ModelError {
    /// The backend (network, inference server) failed. `model` is the model id (`config.llm` or the
    /// embedder's `model`), packed at the `generate`/`embed` boundary so an operator seeing the error
    /// knows which endpoint to investigate. `transient` classifies the failure at construction —
    /// connect/transport failures, timeouts, and HTTP 408/429/5xx are transient (worth retrying);
    /// schema, auth, and other 4xx failures are not.
    Backend {
        model: String,
        message: String,
        transient: bool,
    },
    /// The circuit breaker is open: recent backend calls failed consecutively, so this call failed
    /// fast without reaching the backend (see [`retry::RetryingModel`]).
    CircuitOpen { model: String },
    /// The scripted fake ran out of programmed responses.
    Exhausted,
}

impl ModelError {
    /// Pack the model id into a `Backend` error (the only variant that lacks it at construction time,
    /// because the `backend` helper in `openai.rs` is called from free functions that don't hold `self`).
    /// Called at the `generate`/`embed` boundary so every propagated `Backend` carries the model id.
    pub fn with_model(self, model: &str) -> Self {
        match self {
            ModelError::Backend {
                message, transient, ..
            } => ModelError::Backend {
                model: model.to_owned(),
                message,
                transient,
            },
            other => other,
        }
    }

    /// Whether this failure is transient at the transport level — a retry against the same backend
    /// could succeed. Classified where the backend error is still structured (the `openai` module);
    /// [`retry::RetryingModel`] retries only these.
    pub fn is_transient(&self) -> bool {
        matches!(
            self,
            ModelError::Backend {
                transient: true,
                ..
            }
        )
    }

    /// Whether the model backend is unreachable right now — a transient failure (whose retries, if
    /// any, the wrapper already exhausted) or an open circuit. The condition a routed turn defers
    /// on: the inbound is durable, and the agent catches up when the backend returns.
    pub fn is_unavailable(&self) -> bool {
        self.is_transient() || matches!(self, ModelError::CircuitOpen { .. })
    }
}

impl std::fmt::Display for ModelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ModelError::Backend { model, message, .. } => {
                if model.is_empty() {
                    write!(f, "model: {message}")
                } else {
                    write!(f, "model: {model}: {message}")
                }
            }
            ModelError::CircuitOpen { model } => {
                write!(
                    f,
                    "model: {model}: the circuit is open after repeated backend failures; \
                     failing fast without calling the backend"
                )
            }
            ModelError::Exhausted => {
                write!(
                    f,
                    "model: the scripted model has no more programmed responses"
                )
            }
        }
    }
}

impl std::error::Error for ModelError {}

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

    async fn generate(&self, request: &GenerateRequest) -> Result<GenerateResponse, ModelError> {
        self.seen.lock().push(request.messages.clone());
        self.seen_tool_choice.lock().push(request.tool_choice);
        self.steps.lock().pop_front().ok_or(ModelError::Exhausted)
    }
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

    async fn generate(&self, request: &GenerateRequest) -> Result<GenerateResponse, ModelError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        // Decide under the guard, then release it before delegating: the scripted delegate's
        // `generate` is an await point, and the synchronous lock must not be held across it.
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
            return Err(self.fault());
        }
        self.then.generate(request).await
    }
}

#[cfg(test)]
mod tests {
    //! The scripted model returns its programmed steps in order, then reports exhaustion — the
    //! determinism agent-level tests rely on (spec §Testability).
    use super::{Completion, GenerateRequest, ModelClient, ModelError, ScriptedModel, ToolCall};

    #[test]
    fn generate_request_serializes_with_the_digest_view_shape() {
        // The console's digest verifier mirrors this struct's serialized shape
        // (`RequestDigestView` in `crates/console-wasm`, which pins the identical literal); this
        // canary pins the exact bytes `request_digest` hashes, so a field change here cannot
        // silently break every digest verification.
        assert_eq!(
            serde_json::to_string(&GenerateRequest::default()).unwrap(),
            r#"{"system":"","messages":[],"tools":[],"tool_choice":"Auto","response_format":null,"thinking":null}"#
        );
    }

    #[tokio::test]
    async fn scripted_model_returns_programmed_steps_then_exhausts() {
        let model = ScriptedModel::new([
            Completion::ToolCalls(vec![ToolCall {
                id: "1".to_owned(),
                name: "run_lua".to_owned(),
                arguments: r#"{"script":"return 1"}"#.to_owned(),
            }]),
            Completion::Reply("done".to_owned()),
        ]);
        let request = GenerateRequest::default();

        assert!(matches!(
            model.generate(&request).await.unwrap().completion,
            Completion::ToolCalls(_)
        ));
        assert_eq!(
            model.generate(&request).await.unwrap().completion,
            Completion::Reply("done".to_owned())
        );
        assert!(matches!(
            model.generate(&request).await,
            Err(ModelError::Exhausted)
        ));
    }
}
