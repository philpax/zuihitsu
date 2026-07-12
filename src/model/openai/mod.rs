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

mod request;
mod response;

#[cfg(test)]
mod tests;

use std::time::Duration;

use async_openai::{
    Client,
    config::OpenAIConfig,
    error::OpenAIError,
    types::{
        chat::{
            ChatCompletionStreamOptions, ChatCompletionToolChoiceOption,
            CreateChatCompletionRequestArgs, ToolChoiceOptions,
        },
        embeddings::{CreateEmbeddingRequestArgs, EmbeddingInput},
    },
};
use async_trait::async_trait;
use reqwest::StatusCode;
use serde::Serialize;

use crate::config::{EmbeddingConfig, ModelConfig};

use futures_util::StreamExt;

use super::{
    GenerateDelta, GenerateRequest, GenerateStream, ModelClient, ModelError, ToolChoice,
    embed::{Embedder, Embedding},
};

pub(crate) use request::{ChatRequest, to_messages, to_tools};
pub(crate) use response::{ChatChunk, StreamAssembler};

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
        // `Auto` is the serving layer's default, so it is left unset; `Required` and `None` are
        // mapped explicitly (the loop withdraws the tools on its final step with `None`, forcing a
        // textual answer).
        match request.tool_choice {
            ToolChoice::Auto => {}
            ToolChoice::Required => {
                args.tool_choice(ChatCompletionToolChoiceOption::Mode(
                    ToolChoiceOptions::Required,
                ));
            }
            ToolChoice::None => {
                args.tool_choice(ChatCompletionToolChoiceOption::Mode(
                    ToolChoiceOptions::None,
                ));
            }
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

    /// The one transport: a byot request with `stream: true` and usage requested on the final
    /// chunk. The assembler yields text fragments as they arrive and rebuilds the terminal
    /// [`super::GenerateResponse`] through the same `into_response` mapper the whole-body shape
    /// uses, so the assembled terminal is exactly what a non-streaming request would have returned.
    /// A transport failure mid-stream ends the stream with that error as its last item; the retry
    /// wrapper above decides whether to re-drive.
    async fn generate_stream(&self, request: &GenerateRequest) -> GenerateStream {
        let model = self.config.llm.clone();
        let mut chat_request = match self.build_request(request) {
            Ok(chat_request) => chat_request,
            Err(error) => {
                let error = error.with_model(&model);
                return Box::pin(futures_util::stream::once(async move { Err(error) }));
            }
        };
        chat_request.base.stream = Some(true);
        chat_request.base.stream_options = Some(ChatCompletionStreamOptions {
            include_usage: Some(true),
            include_obfuscation: None,
        });
        let chunks: Result<ChunkStream, OpenAIError> =
            self.client.chat().create_stream_byot(chat_request).await;
        let mut chunks = match chunks {
            Ok(chunks) => chunks,
            Err(error) => {
                let error = backend(error).with_model(&model);
                return Box::pin(futures_util::stream::once(async move { Err(error) }));
            }
        };
        Box::pin(async_stream::stream! {
            let mut assembler = StreamAssembler::default();
            while let Some(chunk) = chunks.next().await {
                match chunk {
                    Ok(chunk) => {
                        for fragment in assembler.fold(chunk) {
                            yield Ok(fragment);
                        }
                    }
                    Err(error) => {
                        yield Err(backend(error).with_model(&model));
                        return;
                    }
                }
            }
            yield assembler
                .finish()
                .map(GenerateDelta::Finished)
                .map_err(|e| e.with_model(&model));
        })
    }
}

/// The byot streaming item feed: chunks as our custom [`ChatChunk`], errors as the crate's.
type ChunkStream =
    std::pin::Pin<Box<dyn futures_util::Stream<Item = Result<ChatChunk, OpenAIError>> + Send>>;

/// An embedder backed by an OpenAI-compatible `/embeddings` endpoint.
pub struct OpenAiEmbedder {
    client: Client<OpenAIConfig>,
    model: String,
    dimensions: usize,
    /// The backend's context window in tokens, when configured — drives the truncation ladder in
    /// [`Self::embed`]. `None` sends inputs whole.
    context_length: Option<usize>,
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
            context_length: config.context_length,
        }
    }

    /// One embeddings request over `inputs` as given, no truncation.
    async fn request(&self, inputs: Vec<String>) -> Result<Vec<Embedding>, ModelError> {
        let request = CreateEmbeddingRequestArgs::default()
            .model(self.model.clone())
            .input(EmbeddingInput::StringArray(inputs))
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

#[async_trait]
impl Embedder for OpenAiEmbedder {
    fn dimensions(&self) -> usize {
        self.dimensions
    }

    fn model_id(&self) -> &str {
        &self.model
    }

    async fn embed(&self, inputs: &[String]) -> Result<Vec<Embedding>, ModelError> {
        let result = match self.context_length {
            Some(context_length) => {
                embed_truncated(inputs, context_length, |clipped| self.request(clipped)).await
            }
            None => self.request(inputs.to_vec()).await,
        };
        result.map_err(|e| e.with_model(&self.model))
    }
}

/// Drive `attempt` down the truncation ladder, keeping every input inside the backend's context
/// window. Inputs are first truncated to 2.5 characters per context token — a deliberately
/// generous ratio, since prose averages fewer tokens than that and over-trimming loses recall
/// signal. The character heuristic can still undershoot the real token count (multi-byte text
/// tokenizes denser), so a rejection that reads as a length overflow trims the character budget by
/// a tenth and retries — a gentle step, since each retry is cheap and every character kept is
/// recall signal — down to a quarter of the window; any other error, or the floor, surfaces as-is.
/// Generic over the attempt so the ladder is testable without an HTTP backend.
async fn embed_truncated<F, Fut>(
    inputs: &[String],
    context_length: usize,
    attempt: F,
) -> Result<Vec<Embedding>, ModelError>
where
    F: Fn(Vec<String>) -> Fut,
    Fut: std::future::Future<Output = Result<Vec<Embedding>, ModelError>>,
{
    let mut budget = context_length * 5 / 2;
    let floor = context_length.div_ceil(4);
    loop {
        let clipped = inputs
            .iter()
            .map(|input| truncate_chars(input, budget))
            .collect();
        match attempt(clipped).await {
            Ok(embeddings) => return Ok(embeddings),
            Err(error) if is_length_overflow(&error) && budget * 9 / 10 >= floor => {
                budget = budget * 9 / 10;
                tracing::debug!(
                    budget,
                    "the embedding backend rejected the batch as over-length; retrying with a smaller truncation"
                );
            }
            Err(error) => return Err(error),
        }
    }
}

/// The leading `max_chars` characters of `text` — a whole-`char` cut, so a multi-byte boundary can
/// never split.
fn truncate_chars(text: &str, max_chars: usize) -> String {
    match text.char_indices().nth(max_chars) {
        Some((cut, _)) => text[..cut].to_owned(),
        None => text.to_owned(),
    }
}

/// Whether a backend rejection reads as the input exceeding the model's context window. The
/// OpenAI-compatible servers do not standardise this failure — llama.cpp says "input is too large
/// to process", vLLM "maximum context length", TEI "must have less than N tokens" — so this matches
/// the recurring vocabulary rather than any one wording. A false negative only skips the retry; a
/// false positive only spends a smaller retry on a request that fails identically.
fn is_length_overflow(error: &ModelError) -> bool {
    let ModelError::Backend { message, .. } = error else {
        return false;
    };
    let message = message.to_lowercase();
    ["context length", "too large", "too long", "token"]
        .iter()
        .any(|needle| message.contains(needle))
}

#[derive(Serialize)]
pub(crate) struct ChatTemplateKwargs {
    enable_thinking: bool,
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
