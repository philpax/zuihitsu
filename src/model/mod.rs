//! The model-client seam: structured text generation. The real client (the local model over the
//! OpenAI-compatible endpoint) is wired in Stage 5; tests use a scripted fake that returns
//! predetermined steps, so an agent-level scenario is deterministic and needs no GPU (spec
//! §Testability). The request/response shape is deliberately small here and grows with the agent
//! loop and tool protocol in Stage 4.
//!
//! This root holds the model-client seam itself; the embedder seam lives in [`embed`], the
//! log-to-vector indexer in [`index`], and the OpenAI-compatible backends for both in [`openai`].

pub mod embed;
pub mod index;
pub mod openai;

use std::{collections::VecDeque, sync::Mutex};

use async_trait::async_trait;

/// A message in the conversation handed to the model. `tool_calls` is populated on an assistant
/// message that called tools; `tool_call_id` ties a tool-result message to the call it answers —
/// the threading the OpenAI protocol needs across multi-step tool use.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    pub role: Role,
    pub content: String,
    pub tool_calls: Vec<ToolCall>,
    pub tool_call_id: Option<String>,
}

impl Message {
    /// An inbound user message.
    pub fn user(content: impl Into<String>) -> Message {
        Message {
            role: Role::User,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: None,
        }
    }

    /// A plain assistant message — an agent turn's reply text replayed into the live buffer (distinct
    /// from [`Message::assistant_tool_calls`], which carries a step's tool calls).
    pub fn assistant(content: impl Into<String>) -> Message {
        Message {
            role: Role::Assistant,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: None,
        }
    }

    /// A system message replayed into the live buffer (a join brief, a time update).
    pub fn system(content: impl Into<String>) -> Message {
        Message {
            role: Role::System,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: None,
        }
    }

    /// The assistant's step that emitted these tool calls.
    pub fn assistant_tool_calls(tool_calls: Vec<ToolCall>) -> Message {
        Message {
            role: Role::Assistant,
            content: String::new(),
            tool_calls,
            tool_call_id: None,
        }
    }

    /// The result of one tool call, answering `tool_call_id`.
    pub fn tool_result(tool_call_id: impl Into<String>, content: impl Into<String>) -> Message {
        Message {
            role: Role::Tool,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: Some(tool_call_id.into()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

/// A tool the model may call: its name, a description, and a JSON-Schema for its arguments, sent to
/// the model so it produces well-formed calls.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// One structured tool call emitted by the model. `arguments` is JSON, parsed by the caller.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

/// How the model may use the available tools. `Auto` lets it choose between a tool call and a reply
/// (the agent loop); `Required` forces it to call a tool, used to coerce structured output — e.g.
/// description regeneration forces a single `describe` tool so the answer can't drift into prose.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ToolChoice {
    #[default]
    Auto,
    Required,
}

/// What the model is asked to produce a step for.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GenerateRequest {
    pub system: String,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolSpec>,
    pub tool_choice: ToolChoice,
    /// Per-request override of the serving layer's reasoning mode: `None` uses the configured
    /// default; `Some(false)` forces it off — used for structured extractions (e.g. description
    /// regeneration), where reasoning adds nothing and makes a forced tool call unreliable.
    pub thinking: Option<bool>,
}

/// A single step's outcome: the model either calls tools or produces a final reply, never both in
/// one step (spec §Agent loop), or it ends the turn silently — a first-class outcome, distinct
/// from an empty reply, for messages not addressed to the agent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Completion {
    ToolCalls(Vec<ToolCall>),
    Reply(String),
    /// End the turn with no reply (the stay-silent terminal).
    Silent,
}

/// The token accounting the serving layer reports for a generation. Fields are `Option` because not
/// every backend returns usage and the scripted fake may decline to script it; an absent
/// `prompt_tokens` makes the compaction trigger fall back to a deterministic estimate over the
/// buffer (spec §Compaction). `prompt_tokens` measures the whole prompt — the frozen prefix plus the
/// live buffer — which is exactly the surface the buffer budget bounds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    pub prompt_tokens: Option<u32>,
}

/// One generation step's result: the [`Completion`] the loop acts on, plus the [`Usage`] the
/// compaction trigger reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GenerateResponse {
    pub completion: Completion,
    pub usage: Usage,
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
    /// The backend (network, inference server) failed.
    Backend(String),
    /// The scripted fake ran out of programmed responses.
    Exhausted,
}

impl std::fmt::Display for ModelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ModelError::Backend(message) => write!(f, "model: {message}"),
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

/// A deterministic fake returning programmed responses in order. Drives agent-loop scenarios
/// without a real model; `generate` records the request's messages (so a test can assert what the
/// model saw — e.g. that a later turn replayed the live buffer), then pops the next scripted step.
pub struct ScriptedModel {
    steps: Mutex<VecDeque<GenerateResponse>>,
    seen: Mutex<Vec<Vec<Message>>>,
}

impl ScriptedModel {
    /// Script the completions a turn will see, each reporting no usage. The common case for scenarios
    /// that don't exercise the compaction trigger.
    pub fn new(steps: impl IntoIterator<Item = Completion>) -> ScriptedModel {
        ScriptedModel::with_responses(steps.into_iter().map(|completion| GenerateResponse {
            completion,
            usage: Usage::default(),
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
                },
            }
        }))
    }

    fn with_responses(steps: impl IntoIterator<Item = GenerateResponse>) -> ScriptedModel {
        ScriptedModel {
            steps: Mutex::new(steps.into_iter().collect()),
            seen: Mutex::new(Vec::new()),
        }
    }

    /// The `messages` of each `generate` call so far, in order — lets a test assert what the model
    /// saw (e.g. that a later turn replayed the prior turns as the prompt suffix).
    pub fn recorded_messages(&self) -> Vec<Vec<Message>> {
        self.seen
            .lock()
            .expect("scripted-model lock poisoned")
            .clone()
    }
}

#[async_trait]
impl ModelClient for ScriptedModel {
    fn model_id(&self) -> &str {
        "scripted-model"
    }

    async fn generate(&self, request: &GenerateRequest) -> Result<GenerateResponse, ModelError> {
        self.seen
            .lock()
            .expect("scripted-model lock poisoned")
            .push(request.messages.clone());
        self.steps
            .lock()
            .expect("scripted-model lock poisoned")
            .pop_front()
            .ok_or(ModelError::Exhausted)
    }
}

#[cfg(test)]
mod tests {
    //! The scripted model returns its programmed steps in order, then reports exhaustion — the
    //! determinism agent-level scenarios rely on (spec §Testability).
    use super::{Completion, GenerateRequest, ModelClient, ModelError, ScriptedModel, ToolCall};

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
