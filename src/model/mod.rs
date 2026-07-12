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
mod fakes;
pub mod index;
pub mod openai;
pub mod priority;
pub mod retry;

pub use fakes::{FlakyModel, ScriptedModel, stream_response};
pub use priority::ModelArbiter;

use async_trait::async_trait;
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

/// One fragment of a streaming generation. Text fragments arrive as the backend produces them; the
/// stream always ends with [`GenerateDelta::Finished`] carrying the same fully-assembled
/// [`GenerateResponse`] the non-streaming path returns, so a consumer that ignores every fragment
/// and keeps only the terminal sees exactly what `generate` would have given it — which is the
/// invariant that keeps the event log byte-identical whether or not anyone watched the tokens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GenerateDelta {
    /// A fragment of the serving layer's `reasoning_content` deliberation.
    Reasoning(String),
    /// A fragment of the reply text.
    Reply(String),
    /// The retry wrapper discarded a partially streamed attempt (a transient failure mid-stream)
    /// and is starting over: everything streamed since the last marker (or the start) is void.
    /// `attempt` is the failed attempt's number, counting from one; `cause` is its failure. A
    /// consumer clears what it accumulated — and the turn loop records the discarded partial as a
    /// `ModelCallAborted` event, so the retry is visible after the fact.
    Restarted { attempt: u32, cause: String },
    /// The stream's end: the complete response, assembled by the client from everything above plus
    /// the pieces that only exist at the end (tool calls, usage, the finish reason).
    Finished(GenerateResponse),
}

/// A boxed stream of generation fragments; an `Err` item ends the stream with the failure.
pub type GenerateStream =
    std::pin::Pin<Box<dyn futures_util::Stream<Item = Result<GenerateDelta, ModelError>> + Send>>;

/// The inference interface. The agent server holds one of these; tests substitute a fake.
///
/// Streaming is the primary and only fundamental operation: every implementation streams, and the
/// unary [`ModelClient::generate`] is a derived convenience that drains the stream to its terminal.
/// There is one transport path, and it is the streamed one — what a live viewer watches, what an
/// eval evaluates, and what a turn records are the same code.
#[async_trait]
pub trait ModelClient: Send + Sync {
    /// The id of the model behind this client, recorded as `produced_by` provenance on the events
    /// its inference produces (spec §Storage → provenance on inference).
    fn model_id(&self) -> &str;

    /// Stream a generation as it is produced: zero or more text fragments (and, from the retry
    /// wrapper, [`GenerateDelta::Restarted`] markers), always ending in a terminal — one
    /// [`GenerateDelta::Finished`] carrying the fully assembled response, or an `Err`. The terminal
    /// is the whole truth: a consumer treats fragments as advisory (display-only) and acts solely
    /// on the terminal, so every consumer behaves identically however the text arrived.
    async fn generate_stream(&self, request: &GenerateRequest) -> GenerateStream;

    /// The drained form: run the stream to its terminal and return it, discarding fragments. For
    /// call sites with no viewer to feed (structured extractions, the judge); never overridden, so
    /// the two forms cannot diverge.
    async fn generate(&self, request: &GenerateRequest) -> Result<GenerateResponse, ModelError> {
        use futures_util::StreamExt as _;
        let mut stream = self.generate_stream(request).await;
        while let Some(delta) = stream.next().await {
            match delta? {
                GenerateDelta::Finished(response) => return Ok(response),
                GenerateDelta::Reasoning(_)
                | GenerateDelta::Reply(_)
                | GenerateDelta::Restarted { .. } => {}
            }
        }
        Err(ModelError::Backend {
            model: self.model_id().to_owned(),
            message: "the stream ended without a terminal response".to_owned(),
            transient: false,
        })
    }
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
