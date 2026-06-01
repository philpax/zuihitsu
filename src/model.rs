//! The model-client seam: structured text generation. The real client (the local model over the
//! OpenAI-compatible endpoint) is wired in Stage 5; tests use a scripted fake that returns
//! predetermined steps, so an agent-level scenario is deterministic and needs no GPU (spec
//! §Testability). The request/response shape is deliberately small here and grows with the agent
//! loop and tool protocol in Stage 4.

use std::{collections::VecDeque, sync::Mutex};

use async_trait::async_trait;

/// A message in the conversation handed to the model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    pub role: Role,
    pub content: String,
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
/// one step (spec §Agent loop). The stay-silent terminal arrives with the loop in Stage 4.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Completion {
    ToolCalls(Vec<ToolCall>),
    Reply(String),
}

/// The inference interface. The agent server holds one of these; tests substitute a fake.
#[async_trait]
pub trait ModelClient: Send + Sync {
    async fn generate(&self, request: &GenerateRequest) -> Result<Completion, ModelError>;
}

/// A model-inference failure. Display messages are lowercase fragments suitable for "failed to {…}".
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
            ModelError::Backend(message) => write!(f, "reach the model backend: {message}"),
            ModelError::Exhausted => {
                write!(
                    f,
                    "produce a model response: the scripted model is exhausted"
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
    async fn generate(&self, _request: &GenerateRequest) -> Result<Completion, ModelError> {
        self.steps
            .lock()
            .expect("scripted-model lock poisoned")
            .pop_front()
            .ok_or(ModelError::Exhausted)
    }
}
