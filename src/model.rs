//! The model-client seam: structured text generation. The real client (the local model over the
//! OpenAI-compatible endpoint) is wired in Stage 5; tests use a scripted fake that returns
//! predetermined steps, so an agent-level scenario is deterministic and needs no GPU (spec
//! §Testability). The request/response shape is deliberately small here and grows with the agent
//! loop and tool protocol in Stage 4.

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

/// A tool the model may call. Stage 4 fills in the real catalogue; for now, a name and description.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
}

/// One structured tool call emitted by the model. `arguments` is JSON, parsed by the caller.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

/// What the model is asked to produce a step for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GenerateRequest {
    pub system: String,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolSpec>,
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

/// The inference interface. The agent server holds one of these; tests substitute a fake.
#[async_trait]
pub trait ModelClient: Send + Sync {
    /// The id of the model behind this client, recorded as `produced_by` provenance on the events
    /// its inference produces (spec §Storage → provenance on inference).
    fn model_id(&self) -> &str;
    async fn generate(&self, request: &GenerateRequest) -> Result<Completion, ModelError>;
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

/// A deterministic fake returning programmed completions in order. Drives agent-loop scenarios
/// without a real model; `generate` ignores the request and pops the next scripted step.
pub struct ScriptedModel {
    steps: Mutex<VecDeque<Completion>>,
}

impl ScriptedModel {
    pub fn new(steps: impl IntoIterator<Item = Completion>) -> ScriptedModel {
        ScriptedModel {
            steps: Mutex::new(steps.into_iter().collect()),
        }
    }
}

#[async_trait]
impl ModelClient for ScriptedModel {
    fn model_id(&self) -> &str {
        "scripted-model"
    }

    async fn generate(&self, _request: &GenerateRequest) -> Result<Completion, ModelError> {
        self.steps
            .lock()
            .expect("scripted-model lock poisoned")
            .pop_front()
            .ok_or(ModelError::Exhausted)
    }
}
