//! The model-interaction wire types: the request/response data the agent loop exchanges with the
//! model, carved out of the host-only model-client seam so the event log (which records what the
//! model saw and produced) and the console (which replays it) can share them in wasm.
//!
//! Only the pure data lives here. The inference interface itself — the `ModelClient` trait, the
//! OpenAI-compatible backends, the scripted test fake, and the `schemars`-driven request builder —
//! stays in the main crate's `model` module, which re-exports these types so they remain reachable
//! at `crate::model::*`.

use serde::{Deserialize, Serialize};

/// A message in the conversation handed to the model. `tool_calls` is populated on an assistant
/// message that called tools; `tool_call_id` ties a tool-result message to the call it answers —
/// the threading the OpenAI protocol needs across multi-step tool use.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

/// A tool the model may call: its name, a description, and a JSON-Schema for its arguments, sent to
/// the model so it produces well-formed calls.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    #[cfg_attr(feature = "ts", ts(type = "any"))]
    pub parameters: serde_json::Value,
}

/// One structured tool call emitted by the model. `arguments` is JSON, parsed by the caller.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

/// How the model may use the available tools. `Auto` lets it choose between a tool call and a reply
/// (the agent loop); `Required` forces it to call a tool, used to coerce structured output — e.g.
/// description regeneration forces a single `describe` tool so the answer can't drift into prose;
/// `None` withdraws the tools so the model must answer in text, used on the agent loop's final step
/// to force a reply out of gathered context rather than spend the last step on another tool call.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum ToolChoice {
    #[default]
    Auto,
    Required,
    None,
}

/// A single step's outcome: the model either calls tools or produces a final reply, never both in
/// one step (spec §Agent loop), or it ends the turn silently — a first-class outcome, distinct
/// from an empty reply, for messages not addressed to the agent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
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
/// live buffer — which is exactly the surface the buffer budget bounds. `completion_tokens` and
/// `total_tokens` are recorded for observability (the model-interaction record) but do not drive the
/// compaction trigger.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Usage {
    pub prompt_tokens: Option<u32>,
    pub completion_tokens: Option<u32>,
    pub total_tokens: Option<u32>,
}
