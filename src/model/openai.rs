//! The real model client and embedder, talking to an OpenAI-compatible HTTP endpoint (spec
//! §Initialization: the endpoint is environmental config). Tests that
//! use it run in a model-gated lane that skips when the endpoint is unreachable.
//!
//! Built on `async-openai`'s types and client. The one thing it can't express is the serving
//! layer's sampling extensions (`top_k`, `min_p`, `chat_template_kwargs`), so the chat request is
//! the sole custom type: a thin wrapper that flattens the standard request and adds those fields,
//! sent via `byot` ("bring your own types"). Everything else — messages, tools, the response, and
//! embeddings — uses async-openai's standard types. A tool's parameter schema travels on its
//! `ToolSpec`. Sampling comes from configuration, not from hardcoded defaults.

use std::time::Duration;

use async_openai::{
    Client,
    config::OpenAIConfig,
    error::OpenAIError,
    types::{
        chat::{
            ChatCompletionMessageToolCall, ChatCompletionMessageToolCalls,
            ChatCompletionRequestAssistantMessageArgs, ChatCompletionRequestMessage,
            ChatCompletionRequestSystemMessageArgs, ChatCompletionRequestToolMessageArgs,
            ChatCompletionRequestUserMessageArgs, ChatCompletionTool,
            ChatCompletionToolChoiceOption, ChatCompletionTools, CreateChatCompletionRequest,
            CreateChatCompletionRequestArgs, FunctionCall, FunctionObject, ToolChoiceOptions,
        },
        embeddings::{CreateEmbeddingRequestArgs, EmbeddingInput},
    },
};
use async_trait::async_trait;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};

use crate::config::{EmbeddingConfig, ModelConfig};

use super::{
    Completion, GenerateRequest, GenerateResponse, ModelClient, ModelError, Role, ToolCall,
    ToolChoice, Usage,
    embed::{Embedder, Embedding},
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
            client: client(
                &config.endpoint,
                Duration::from_secs(config.resilience.request_timeout_seconds),
            ),
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

        // A response-format constraint is built as raw JSON for the byot path: the response-format
        // grammar path is the one some serving layers actually schema-constrain (where forced-tool-call
        // arguments are not), so a single structured extraction goes here rather than through a tool.
        let response_format = request.response_format.as_ref().map(|format| {
            serde_json::json!({
                "type": "json_schema",
                "json_schema": {
                    "name": format.name,
                    "schema": format.schema,
                    "strict": true,
                },
            })
        });

        Ok(ChatRequest {
            base,
            response_format,
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
        let model = &self.config.llm;
        let result: Result<GenerateResponse, ModelError> = async {
            let response: ChatResponse = self
                .client
                .chat()
                .create_byot(self.build_request(request)?)
                .await
                .map_err(backend)?;
            into_response(response)
        }
        .await;
        result.map_err(|e| e.with_model(model))
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
            client: client(
                &config.endpoint,
                Duration::from_secs(config.request_timeout_seconds),
            ),
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
        let model = &self.model;
        let result: Result<Vec<Embedding>, ModelError> = async {
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
        .await;
        result.map_err(|e| e.with_model(model))
    }
}

/// The one custom type: the standard chat request plus the serving layer's sampling extensions,
/// which the standard schema does not model. Sent through `create_byot`.
#[derive(Serialize)]
struct ChatRequest {
    #[serde(flatten)]
    base: CreateChatCompletionRequest,
    /// An OpenAI `response_format` object, built as raw JSON for the byot path — a json-schema
    /// constraint on the whole reply (see [`crate::model::ResponseSchema`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<serde_json::Value>,
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

/// The custom response type. `async-openai`'s `CreateChatCompletionResponse` drops the serving
/// layer's `reasoning_content` (it has no such field), so the deliberation the model-interaction
/// record needs would be lost. This minimal mirror keeps the choice/message/usage shape and adds
/// `reasoning_content`, surfaced through `create_byot`. The dependence on the serving layer's
/// `reasoning_content` convention (vLLM/qwen) is an accepted, contained coupling: an endpoint that
/// names the field differently simply yields `reasoning: None`, never an error.
#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
    #[serde(default)]
    usage: Option<ChatUsage>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatMessage,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct ChatMessage {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    reasoning_content: Option<String>,
    #[serde(default)]
    tool_calls: Vec<ChatToolCall>,
}

#[derive(Deserialize)]
struct ChatToolCall {
    id: String,
    function: ChatFunctionCall,
}

#[derive(Deserialize)]
struct ChatFunctionCall {
    name: String,
    arguments: String,
}

#[derive(Deserialize)]
struct ChatUsage {
    prompt_tokens: u32,
    #[serde(default)]
    completion_tokens: Option<u32>,
    #[serde(default)]
    total_tokens: Option<u32>,
}

/// Build the HTTP client for an endpoint with a whole-request `timeout`. reqwest's default is *no*
/// timeout, so without one a hung backend stalls its caller forever; with it, the stall surfaces as
/// a retryable timeout error. The panic on a client-build failure mirrors `reqwest::Client::new`
/// (the path taken before this timeout existed): it fires only when the TLS backend cannot
/// initialize, a build/environment defect, not a runtime condition.
fn client(endpoint: &str, timeout: Duration) -> Client<OpenAIConfig> {
    let http_client = reqwest::Client::builder()
        .timeout(timeout)
        .build()
        .expect("the HTTP client builds; only TLS-backend initialization can fail here");
    Client::build(
        http_client,
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

/// Map the custom byot response into a [`GenerateResponse`], surfacing the deliberation the standard
/// type drops. Kept separate from `generate` so it can be tested over a fixture body without a live
/// endpoint.
fn into_response(response: ChatResponse) -> Result<GenerateResponse, ModelError> {
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

fn into_completion(message: ChatMessage) -> Completion {
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

/// Map any backend error (async-openai's `OpenAIError` from the client or the request builders)
/// into our model error, classifying it as transient or not while the error is still structured.
/// The model id is packed in at the `generate`/`embed` boundary via [`ModelError::with_model`],
/// because this helper is called from free functions that don't hold `self`.
fn backend(error: OpenAIError) -> ModelError {
    ModelError::Backend {
        model: String::new(),
        transient: is_transient(&error),
        message: error.to_string(),
    }
}

/// Whether a backend failure is transient at the transport level — worth retrying against the same
/// endpoint. Transport failures (could not connect, timed out, the connection dropped mid-body) and
/// overload/server statuses (408, 429, 5xx) are transient; everything else — request-builder and
/// serde failures, auth and other 4xx statuses — reflects the request or the configuration and
/// retries identically, so it is not.
fn is_transient(error: &OpenAIError) -> bool {
    match error {
        OpenAIError::Reqwest(error) => error.is_connect() || error.is_timeout() || error.is_body(),
        OpenAIError::ApiError(response) => {
            let status = response.status_code;
            status == StatusCode::REQUEST_TIMEOUT
                || status == StatusCode::TOO_MANY_REQUESTS
                || status.is_server_error()
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    //! The custom byot response type must deserialize the serving layer's `reasoning_content` and
    //! the full token usage that the standard `async-openai` type drops — the deliberation the
    //! model-interaction record captures (spec §Observability) — and the backend-error
    //! classification must mark exactly the transport/overload failures transient.
    use async_openai::error::{ApiError, ApiErrorResponse, OpenAIError};
    use reqwest::StatusCode;

    use super::{ChatResponse, into_response, is_transient};
    use crate::model::{Completion, Usage};

    fn api_error(status: StatusCode) -> OpenAIError {
        OpenAIError::ApiError(ApiErrorResponse {
            status_code: status,
            api_error: ApiError {
                message: "backend says no".to_owned(),
                r#type: None,
                param: None,
                code: None,
            },
        })
    }

    #[test]
    fn classifies_api_statuses() {
        // Overload and server-side statuses are transient; request-side statuses are not.
        for (status, expected) in [
            (StatusCode::REQUEST_TIMEOUT, true),
            (StatusCode::TOO_MANY_REQUESTS, true),
            (StatusCode::INTERNAL_SERVER_ERROR, true),
            (StatusCode::BAD_GATEWAY, true),
            (StatusCode::SERVICE_UNAVAILABLE, true),
            (StatusCode::BAD_REQUEST, false),
            (StatusCode::UNAUTHORIZED, false),
            (StatusCode::NOT_FOUND, false),
            (StatusCode::UNPROCESSABLE_ENTITY, false),
        ] {
            assert_eq!(
                is_transient(&api_error(status)),
                expected,
                "misclassified {status}"
            );
        }
    }

    #[test]
    fn builder_and_serde_failures_are_not_transient() {
        assert!(!is_transient(&OpenAIError::InvalidArgument(
            "missing model".to_owned()
        )));
        let serde_error = serde_json::from_str::<serde_json::Value>("not json").unwrap_err();
        assert!(!is_transient(&OpenAIError::JSONDeserialize(
            serde_error,
            "not json".to_owned()
        )));
    }

    #[test]
    fn deserializes_reasoning_and_full_usage_from_a_reply() {
        let body = serde_json::json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": "  Hello there.\n",
                    "reasoning_content": "The user greeted me, so I greet back."
                },
                "finish_reason": "stop"
            }],
            "usage": { "prompt_tokens": 12, "completion_tokens": 5, "total_tokens": 17 }
        });
        let response: ChatResponse = serde_json::from_value(body).expect("body deserializes");
        let generated = into_response(response).expect("maps to a response");

        assert_eq!(
            generated.completion,
            Completion::Reply("Hello there.".to_owned())
        );
        assert_eq!(
            generated.reasoning.as_deref(),
            Some("The user greeted me, so I greet back.")
        );
        assert_eq!(generated.finish_reason.as_deref(), Some("stop"));
        assert_eq!(
            generated.usage,
            Usage {
                prompt_tokens: Some(12),
                completion_tokens: Some(5),
                total_tokens: Some(17),
            }
        );
    }

    #[test]
    fn deserializes_a_tool_call_and_tolerates_absent_reasoning() {
        let body = serde_json::json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": { "name": "run_lua", "arguments": "{\"script\":\"return 1\"}" }
                    }]
                },
                "finish_reason": "tool_calls"
            }]
        });
        let response: ChatResponse = serde_json::from_value(body).expect("body deserializes");
        let generated = into_response(response).expect("maps to a response");

        match generated.completion {
            Completion::ToolCalls(calls) => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].name, "run_lua");
            }
            other => panic!("expected a tool call, got {other:?}"),
        }
        // No `reasoning_content` and no `usage` block — both degrade to `None`, never an error.
        assert_eq!(generated.reasoning, None);
        assert_eq!(generated.usage, Usage::default());
        assert_eq!(generated.finish_reason.as_deref(), Some("tool_calls"));
    }
}
