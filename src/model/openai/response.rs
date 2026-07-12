//! The custom byot response types: the choice/message/usage shape that surfaces the serving layer's
//! `reasoning_content` — in both its whole-body and streaming-chunk forms — plus the mappers into
//! [`GenerateResponse`] and [`Completion`]. The streaming assembler folds chunks back into a
//! [`ChatResponse`] and reuses [`into_response`], so the two transports cannot map differently.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::model::{Completion, GenerateDelta, GenerateResponse, ModelError, ToolCall, Usage};

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
    #[serde(default)]
    pub(crate) prompt_tokens_details: Option<PromptTokensDetails>,
}

/// The cached-token breakdown some OpenAI-compatible servers attach to usage. llama.cpp's
/// compatibility endpoint omits it entirely; an absent field means the cache behavior is unknown,
/// never that zero tokens were cached.
#[derive(Deserialize)]
pub(crate) struct PromptTokensDetails {
    #[serde(default)]
    pub(crate) cached_tokens: Option<u32>,
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
        cache_read_tokens: response
            .usage
            .as_ref()
            .and_then(|usage| usage.prompt_tokens_details.as_ref())
            .and_then(|details| details.cached_tokens),
        cache_write_tokens: None,
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

/// One server-sent chunk of a streaming completion: the delta form of [`ChatResponse`]. The final
/// chunk carries `usage` when the request set `stream_options.include_usage`; tool-call arguments
/// arrive as fragments keyed by `index`, concatenated by the assembler.
#[derive(Deserialize)]
pub(crate) struct ChatChunk {
    #[serde(default)]
    pub(crate) choices: Vec<ChunkChoice>,
    #[serde(default)]
    pub(crate) usage: Option<ChatUsage>,
}

#[derive(Deserialize)]
pub(crate) struct ChunkChoice {
    #[serde(default)]
    pub(crate) delta: ChunkDelta,
    #[serde(default)]
    pub(crate) finish_reason: Option<String>,
}

#[derive(Default, Deserialize)]
pub(crate) struct ChunkDelta {
    #[serde(default)]
    pub(crate) content: Option<String>,
    #[serde(default)]
    pub(crate) reasoning_content: Option<String>,
    #[serde(default)]
    pub(crate) tool_calls: Vec<ChunkToolCall>,
}

#[derive(Deserialize)]
pub(crate) struct ChunkToolCall {
    /// Which of the message's tool calls this fragment extends — fragments of one call share an
    /// index, and `id`/`name` arrive on its first fragment only.
    #[serde(default)]
    pub(crate) index: u32,
    #[serde(default)]
    pub(crate) id: Option<String>,
    #[serde(default)]
    pub(crate) function: Option<ChunkFunction>,
}

#[derive(Deserialize)]
pub(crate) struct ChunkFunction {
    #[serde(default)]
    pub(crate) name: Option<String>,
    #[serde(default)]
    pub(crate) arguments: Option<String>,
}

/// Folds streaming chunks into the whole response. `fold` returns the display fragments a chunk
/// contributes; `finish` rebuilds a [`ChatResponse`] from the accumulation and maps it through the
/// same [`into_response`] as the non-streaming path, so the terminal result is identical to what a
/// whole-body request would have produced — the invariant the recorded `ModelCalled` relies on.
#[derive(Default)]
pub(crate) struct StreamAssembler {
    content: String,
    reasoning: String,
    /// Tool calls under assembly, keyed by the chunk `index`; argument fragments concatenate.
    tool_calls: BTreeMap<u32, PartialToolCall>,
    finish_reason: Option<String>,
    usage: Option<ChatUsage>,
    /// Whether any chunk arrived at all: a body with zero chunks is the backend closing without
    /// producing anything, surfaced as an error rather than assembled into an empty reply.
    saw_chunk: bool,
}

#[derive(Default)]
struct PartialToolCall {
    id: String,
    name: String,
    arguments: String,
}

impl StreamAssembler {
    /// Absorb one chunk, returning the fragments a live viewer should see now.
    pub(crate) fn fold(&mut self, chunk: ChatChunk) -> Vec<GenerateDelta> {
        self.saw_chunk = true;
        let mut fragments = Vec::new();
        if let Some(usage) = chunk.usage {
            self.usage = Some(usage);
        }
        for choice in chunk.choices {
            if let Some(reason) = choice.finish_reason {
                self.finish_reason = Some(reason);
            }
            if let Some(reasoning) = choice.delta.reasoning_content
                && !reasoning.is_empty()
            {
                self.reasoning.push_str(&reasoning);
                fragments.push(GenerateDelta::Reasoning(reasoning));
            }
            if let Some(content) = choice.delta.content
                && !content.is_empty()
            {
                self.content.push_str(&content);
                fragments.push(GenerateDelta::Reply(content));
            }
            for call in choice.delta.tool_calls {
                let partial = self.tool_calls.entry(call.index).or_default();
                if let Some(id) = call.id {
                    partial.id = id;
                }
                if let Some(function) = call.function {
                    if let Some(name) = function.name {
                        partial.name = name;
                    }
                    if let Some(arguments) = function.arguments {
                        partial.arguments.push_str(&arguments);
                    }
                }
            }
        }
        fragments
    }

    /// The assembled terminal response, mapped through the shared non-streaming mapper.
    pub(crate) fn finish(self) -> Result<GenerateResponse, ModelError> {
        let StreamAssembler {
            content,
            reasoning,
            tool_calls,
            finish_reason,
            usage,
            saw_chunk,
        } = self;
        if !saw_chunk {
            return Err(ModelError::Backend {
                model: String::new(),
                message: "the stream closed without a single chunk".to_owned(),
                // The connection worked but the serving layer produced nothing; a retry of the
                // identical request is worth attempting.
                transient: true,
            });
        }
        into_response(ChatResponse {
            choices: vec![ChatChoice {
                message: ChatMessage {
                    content: Some(content),
                    reasoning_content: (!reasoning.is_empty()).then_some(reasoning),
                    tool_calls: tool_calls
                        .into_values()
                        .map(|call| ChatToolCall {
                            id: call.id,
                            function: ChatFunctionCall {
                                name: call.name,
                                arguments: call.arguments,
                            },
                        })
                        .collect(),
                },
                finish_reason,
            }],
            usage,
        })
    }
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
