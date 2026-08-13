//! The model-interaction wire types: the request/response data the agent loop exchanges with the
//! model, carved out of the host-only model-client seam so the event log (which records what the
//! model saw and produced) and the console (which replays it) can share them in wasm.
//!
//! Only the pure data lives here. The inference interface itself — the `ModelClient` trait, the
//! OpenAI-compatible backends, the scripted test fake, and the `schemars`-driven request builder —
//! stays in the main crate's `model` module, which re-exports these types so they remain reachable
//! at `crate::model::*`.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::ids::BlobHash;

/// Characters per token in the deterministic fallback used when the provider reports no usage.
const ESTIMATED_CHARS_PER_TOKEN: usize = 4;

/// Estimate tokens from a Unicode scalar-value count using the shared fallback rule. A non-empty
/// fragment rounds up so the fallback does not understate the prompt budget.
pub fn estimated_tokens_from_chars(chars: usize) -> usize {
    chars.div_ceil(ESTIMATED_CHARS_PER_TOKEN)
}

/// Estimate tokens from text using the shared fallback rule.
pub fn estimated_tokens(text: &str) -> usize {
    estimated_tokens_from_chars(text.chars().count())
}

#[cfg(test)]
mod token_estimate_tests {
    use super::{estimated_tokens, estimated_tokens_from_chars};

    #[test]
    fn uses_unicode_scalar_counts_and_ceiling_division() {
        assert_eq!(estimated_tokens_from_chars(0), 0);
        assert_eq!(estimated_tokens_from_chars(3), 1);
        assert_eq!(estimated_tokens_from_chars(4), 1);
        assert_eq!(estimated_tokens("🐚🐚🐚🐚"), 1);
        assert_eq!(estimated_tokens("🐚🐚🐚"), 1);
    }
}

/// A message in the conversation handed to the model. `tool_calls` is populated on an assistant
/// message that called tools; `tool_call_id` ties a tool-result message to the call it answers —
/// the threading the OpenAI protocol needs across multi-step tool use.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Message {
    pub role: Role,
    pub content: String,
    /// The images this message shows the model, alongside its text — the perceivable attachments a
    /// participant's turn carried. Skipped from serialisation when empty, so a text-only message
    /// hashes to exactly the bytes it always did and every `request_digest` recorded to date stays
    /// verifiable.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<ImagePart>,
    pub tool_calls: Vec<ToolCall>,
    pub tool_call_id: Option<String>,
}

/// One image shown to the model. The `blob` address is the content identity, so it alone is what the
/// request digest and a captured `ModelCalled` record need; the bytes themselves are held beside it
/// only for the trip to the backend.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ImagePart {
    /// The content address of the image's bytes — what identifies this image in a digest, and the
    /// key the console fetches it under.
    pub blob: BlobHash,
    /// The media type the bytes were stored under, which the `data:` URI declares to the backend.
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub mime: SmolStr,
    /// The base64 payload sent to the backend, shared across a turn's steps rather than re-encoded
    /// per step. Deliberately outside serialisation: the address above already identifies the
    /// content, so a digest stays a digest and a captured request record stays small rather than
    /// carrying megabytes of base64 into the event log.
    #[serde(skip)]
    #[cfg_attr(feature = "ts", ts(skip))]
    pub data: Arc<str>,
}

impl Message {
    /// An inbound user message.
    pub fn user(content: impl Into<String>) -> Message {
        Message {
            role: Role::User,
            images: Vec::new(),
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
            images: Vec::new(),
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: None,
        }
    }

    /// A system message replayed into the live buffer (a join brief, a time update).
    pub fn system(content: impl Into<String>) -> Message {
        Message {
            role: Role::System,
            images: Vec::new(),
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: None,
        }
    }

    /// The assistant's step that emitted these tool calls.
    pub fn assistant_tool_calls(tool_calls: Vec<ToolCall>) -> Message {
        Message {
            role: Role::Assistant,
            images: Vec::new(),
            content: String::new(),
            tool_calls,
            tool_call_id: None,
        }
    }

    /// The result of one tool call, answering `tool_call_id`.
    pub fn tool_result(tool_call_id: impl Into<String>, content: impl Into<String>) -> Message {
        Message {
            role: Role::Tool,
            images: Vec::new(),
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
    /// Prompt tokens the provider served from its prefix cache. `None` when the provider does not
    /// report cache usage — unknown, not zero.
    #[serde(default)]
    pub cache_read_tokens: Option<u32>,
    /// Prompt tokens the provider wrote to its cache. `None` when unreported; no OpenAI-compatible
    /// server emits a write signal today, so this exists for providers that do.
    #[serde(default)]
    pub cache_write_tokens: Option<u32>,
}
