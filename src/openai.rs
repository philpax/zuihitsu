//! The real model client and embedder, talking to an OpenAI-compatible HTTP endpoint (spec
//! §Initialization: the endpoint is environmental config). Behind the `openai` feature; tests that
//! use it run in a model-gated lane that skips when the endpoint is unreachable.
//!
//! Built on `async-openai`'s types and client. The one thing it can't express is the serving
//! layer's sampling extensions (`top_k`, `min_p`, `chat_template_kwargs`), so the chat request is
//! the sole custom type: a thin wrapper that flattens the standard request and adds those fields,
//! sent via `byot` ("bring your own types"). Everything else — messages, tools, the response, and
//! embeddings — uses async-openai's standard types. A tool's parameter schema travels on its
//! `ToolSpec`. Sampling comes from configuration, not from hardcoded defaults.

use async_openai::{
    Client,
    config::OpenAIConfig,
    types::{
        chat::{
            ChatCompletionMessageToolCall, ChatCompletionMessageToolCalls,
            ChatCompletionRequestAssistantMessageArgs, ChatCompletionRequestMessage,
            ChatCompletionRequestSystemMessageArgs, ChatCompletionRequestToolMessageArgs,
            ChatCompletionRequestUserMessageArgs, ChatCompletionResponseMessage,
            ChatCompletionTool, ChatCompletionToolChoiceOption, ChatCompletionTools,
            CreateChatCompletionRequest, CreateChatCompletionRequestArgs,
            CreateChatCompletionResponse, FunctionCall, FunctionObject, ToolChoiceOptions,
        },
        embeddings::{CreateEmbeddingRequestArgs, EmbeddingInput},
    },
};
use async_trait::async_trait;
use serde::Serialize;

use crate::{
    config::{EmbeddingConfig, ModelConfig},
    embed::{Embedder, Embedding},
    model::{
        Completion, GenerateRequest, GenerateResponse, ModelClient, ModelError, Role, ToolCall,
        ToolChoice, Usage,
    },
};

/// A generation client backed by an OpenAI-compatible `/chat/completions` endpoint. Holds the
/// `[model]` config, which carries both the model name and the (optional) sampling parameters.
pub struct OpenAiClient {
    client: Client<OpenAIConfig>,
    config: ModelConfig,
}

impl OpenAiClient {
    pub fn new(config: &ModelConfig) -> OpenAiClient {
        OpenAiClient {
            client: client(&config.endpoint),
            config: config.clone(),
        }
    }

    fn build_request(&self, request: &GenerateRequest) -> Result<ChatRequest, ModelError> {
        let mut args = CreateChatCompletionRequestArgs::default();
        args.model(self.config.llm.clone());
        args.messages(to_messages(request)?);
        let tools = to_tools(request);
        if !tools.is_empty() {
            args.tools(tools);
        }
        if request.tool_choice == ToolChoice::Required {
            args.tool_choice(ChatCompletionToolChoiceOption::Mode(
                ToolChoiceOptions::Required,
            ));
        }
        if let Some(temperature) = self.config.temperature {
            args.temperature(temperature);
        }
        if let Some(top_p) = self.config.top_p {
            args.top_p(top_p);
        }
        if let Some(presence_penalty) = self.config.presence_penalty {
            args.presence_penalty(presence_penalty);
        }
        let base = args.build().map_err(backend)?;

        Ok(ChatRequest {
            base,
            top_k: self.config.top_k,
            min_p: self.config.min_p,
            // A per-request `thinking` overrides the configured default (e.g. regeneration forces
            // reasoning off).
            chat_template_kwargs: request
                .thinking
                .or(self.config.thinking)
                .map(|enable_thinking| ChatTemplateKwargs { enable_thinking }),
        })
    }
}

#[async_trait]
impl ModelClient for OpenAiClient {
    fn model_id(&self) -> &str {
        &self.config.llm
    }

    async fn generate(&self, request: &GenerateRequest) -> Result<GenerateResponse, ModelError> {
        let response: CreateChatCompletionResponse = self
            .client
            .chat()
            .create_byot(self.build_request(request)?)
            .await
            .map_err(backend)?;
        // Read usage before `choices` is moved out: `prompt_tokens` covers the whole prompt — the
        // frozen prefix plus the live buffer — which is what the compaction budget bounds.
        let usage = Usage {
            prompt_tokens: response.usage.as_ref().map(|usage| usage.prompt_tokens),
        };
        let message = response
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| ModelError::Backend("response contained no choices".to_owned()))?
            .message;
        Ok(GenerateResponse {
            completion: into_completion(message),
            usage,
        })
    }
}

/// An embedder backed by an OpenAI-compatible `/embeddings` endpoint (jina v5 in our deployment).
pub struct OpenAiEmbedder {
    client: Client<OpenAIConfig>,
    model: String,
    dimensions: usize,
}

impl OpenAiEmbedder {
    /// Build an embedder from the `[embedding]` config, which carries the endpoint, the model, and
    /// the dimensionality it produces (so callers can size the vector store without a probe).
    pub fn new(config: &EmbeddingConfig) -> OpenAiEmbedder {
        OpenAiEmbedder {
            client: client(&config.endpoint),
            model: config.model.clone(),
            dimensions: config.dimensions,
        }
    }
}

#[async_trait]
impl Embedder for OpenAiEmbedder {
    fn dimensions(&self) -> usize {
        self.dimensions
    }

    fn model_id(&self) -> &str {
        &self.model
    }

    async fn embed(&self, inputs: &[String]) -> Result<Vec<Embedding>, ModelError> {
        let request = CreateEmbeddingRequestArgs::default()
            .model(self.model.clone())
            .input(EmbeddingInput::StringArray(inputs.to_vec()))
            .build()
            .map_err(backend)?;
        let response = self
            .client
            .embeddings()
            .create(request)
            .await
            .map_err(backend)?;
        Ok(response
            .data
            .into_iter()
            .map(|datum| datum.embedding)
            .collect())
    }
}

/// The one custom type: the standard chat request plus the serving layer's sampling extensions,
/// which the standard schema does not model. Sent through `create_byot`.
#[derive(Serialize)]
struct ChatRequest {
    #[serde(flatten)]
    base: CreateChatCompletionRequest,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_k: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    min_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    chat_template_kwargs: Option<ChatTemplateKwargs>,
}

#[derive(Serialize)]
struct ChatTemplateKwargs {
    enable_thinking: bool,
}

fn client(endpoint: &str) -> Client<OpenAIConfig> {
    Client::with_config(
        OpenAIConfig::new()
            .with_api_base(endpoint.trim_end_matches('/'))
            .with_api_key("unused"),
    )
}

fn to_messages(request: &GenerateRequest) -> Result<Vec<ChatCompletionRequestMessage>, ModelError> {
    fn system_message(content: &str) -> Result<ChatCompletionRequestMessage, ModelError> {
        Ok(ChatCompletionRequestSystemMessageArgs::default()
            .content(content.to_owned())
            .build()
            .map_err(backend)?
            .into())
    }

    fn message_tool_call(call: &ToolCall) -> ChatCompletionMessageToolCalls {
        ChatCompletionMessageToolCalls::Function(ChatCompletionMessageToolCall {
            id: call.id.clone(),
            function: FunctionCall {
                name: call.name.clone(),
                arguments: call.arguments.clone(),
            },
        })
    }

    let mut messages = Vec::with_capacity(request.messages.len() + 1);
    if !request.system.is_empty() {
        messages.push(system_message(&request.system)?);
    }
    for message in &request.messages {
        messages.push(match message.role {
            Role::System => system_message(&message.content)?,
            Role::User => ChatCompletionRequestUserMessageArgs::default()
                .content(message.content.clone())
                .build()
                .map_err(backend)?
                .into(),
            Role::Assistant => {
                let mut args = ChatCompletionRequestAssistantMessageArgs::default();
                if !message.content.is_empty() {
                    args.content(message.content.clone());
                }
                if !message.tool_calls.is_empty() {
                    args.tool_calls(
                        message
                            .tool_calls
                            .iter()
                            .map(message_tool_call)
                            .collect::<Vec<_>>(),
                    );
                }
                args.build().map_err(backend)?.into()
            }
            Role::Tool => ChatCompletionRequestToolMessageArgs::default()
                .content(message.content.clone())
                .tool_call_id(message.tool_call_id.clone().unwrap_or_default())
                .build()
                .map_err(backend)?
                .into(),
        });
    }
    Ok(messages)
}

fn to_tools(request: &GenerateRequest) -> Vec<ChatCompletionTools> {
    request
        .tools
        .iter()
        .map(|tool| {
            ChatCompletionTools::Function(ChatCompletionTool {
                function: FunctionObject {
                    name: tool.name.clone(),
                    description: Some(tool.description.clone()),
                    parameters: Some(tool.parameters.clone()),
                    strict: None,
                },
            })
        })
        .collect()
}

fn into_completion(message: ChatCompletionResponseMessage) -> Completion {
    fn response_tool_call(call: ChatCompletionMessageToolCalls) -> Option<ToolCall> {
        match call {
            ChatCompletionMessageToolCalls::Function(call) => Some(ToolCall {
                id: call.id,
                name: call.function.name,
                arguments: call.function.arguments,
            }),
            // Custom (free-form) tool calls aren't part of our protocol.
            ChatCompletionMessageToolCalls::Custom(_) => None,
        }
    }

    match message.tool_calls {
        Some(calls) if !calls.is_empty() => {
            Completion::ToolCalls(calls.into_iter().filter_map(response_tool_call).collect())
        }
        // Trim surrounding whitespace: with thinking on, the content after the reasoning block
        // arrives with leading newlines from the template's reasoning/content boundary, and that
        // would otherwise be recorded verbatim in the durable turn.
        _ => Completion::Reply(message.content.unwrap_or_default().trim().to_owned()),
    }
}

/// Map any backend error (async-openai's `OpenAIError` from the client or the request builders)
/// into our model error.
fn backend(error: impl std::fmt::Display) -> ModelError {
    ModelError::Backend(error.to_string())
}
