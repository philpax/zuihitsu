//! The custom byot response type: the choice/message/usage shape that surfaces the serving layer's
//! `reasoning_content`, plus the mappers into [`GenerateResponse`] and [`Completion`].

use serde::Deserialize;

use crate::model::{Completion, GenerateResponse, ModelError, ToolCall, Usage};

/// The custom response type. `async-openai`'s `CreateChatCompletionResponse` drops the serving
/// layer's `reasoning_content` (it has no such field), so the deliberation the model-interaction
/// record needs would be lost. This minimal mirror keeps the choice/message/usage shape and adds
/// `reasoning_content`, surfaced through `create_byot`. The dependence on the serving layer's
/// `reasoning_content` convention (vLLM/qwen) is an accepted, contained coupling: an endpoint that
/// names the field differently simply yields `reasoning: None`, never an error.
#[derive(Deserialize)]
pub(crate) struct ChatResponse {
    pub(crate) choices: Vec<ChatChoice>,
    #[serde(default)]
    pub(crate) usage: Option<ChatUsage>,
}

#[derive(Deserialize)]
pub(crate) struct ChatChoice {
    pub(crate) message: ChatMessage,
    #[serde(default)]
    pub(crate) finish_reason: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct ChatMessage {
    #[serde(default)]
    pub(crate) content: Option<String>,
    #[serde(default)]
    pub(crate) reasoning_content: Option<String>,
    #[serde(default)]
    pub(crate) tool_calls: Vec<ChatToolCall>,
}

#[derive(Deserialize)]
pub(crate) struct ChatToolCall {
    pub(crate) id: String,
    pub(crate) function: ChatFunctionCall,
}

#[derive(Deserialize)]
pub(crate) struct ChatFunctionCall {
    pub(crate) name: String,
    pub(crate) arguments: String,
}

#[derive(Deserialize)]
pub(crate) struct ChatUsage {
    pub(crate) prompt_tokens: u32,
    #[serde(default)]
    pub(crate) completion_tokens: Option<u32>,
    #[serde(default)]
    pub(crate) total_tokens: Option<u32>,
}

/// Map the custom byot response into a [`GenerateResponse`], surfacing the deliberation the standard
/// type drops. Kept separate from `generate` so it can be tested over a fixture body without a live
/// endpoint.
pub(crate) fn into_response(response: ChatResponse) -> Result<GenerateResponse, ModelError> {
    // Read usage before the choice is moved out: `prompt_tokens` covers the whole prompt — the
    // frozen prefix plus the live buffer — which is what the compaction budget bounds;
    // `completion_tokens`/`total_tokens` ride along only for the model-interaction record.
    let usage = Usage {
        prompt_tokens: response.usage.as_ref().map(|usage| usage.prompt_tokens),
        completion_tokens: response
            .usage
            .as_ref()
            .and_then(|usage| usage.completion_tokens),
        total_tokens: response.usage.as_ref().and_then(|usage| usage.total_tokens),
    };
    let ChatChoice {
        message,
        finish_reason,
    } = response
        .choices
        .into_iter()
        .next()
        .ok_or_else(|| ModelError::Backend {
            model: String::new(),
            message: "response contained no choices".to_owned(),
            // A well-formed but empty response is a serving-layer contract violation, not a
            // transport blip — retrying the same request is not expected to change it.
            transient: false,
        })?;
    let reasoning = message
        .reasoning_content
        .clone()
        .filter(|reasoning| !reasoning.is_empty());
    Ok(GenerateResponse {
        completion: into_completion(message),
        usage,
        reasoning,
        finish_reason,
    })
}

pub(crate) fn into_completion(message: ChatMessage) -> Completion {
    if !message.tool_calls.is_empty() {
        return Completion::ToolCalls(
            message
                .tool_calls
                .into_iter()
                .map(|call| ToolCall {
                    id: call.id,
                    name: call.function.name,
                    arguments: call.function.arguments,
                })
                .collect(),
        );
    }
    // Trim surrounding whitespace: with thinking on, the content after the reasoning block arrives
    // with leading newlines from the template's reasoning/content boundary, and that would
    // otherwise be recorded verbatim in the durable turn.
    Completion::Reply(message.content.unwrap_or_default().trim().to_owned())
}
