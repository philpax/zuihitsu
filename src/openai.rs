//! The real model client and embedder, talking to an OpenAI-compatible HTTP endpoint (spec
//! §Initialization: the endpoint is environmental config). Behind the `openai` feature; tests that
//! use it run in a model-gated lane that skips when the endpoint is unreachable.
//!
//! Only the embedder lives here for now; the generation client follows in the next increment.

use async_trait::async_trait;

use crate::{
    embed::{Embedder, Embedding},
    model::ModelError,
};

/// An embedder backed by an OpenAI-compatible `/embeddings` endpoint (jina v5 in our deployment).
pub struct OpenAiEmbedder {
    http: reqwest::Client,
    embeddings_url: String,
    model: String,
    dimensions: usize,
}

impl OpenAiEmbedder {
    /// `endpoint` is the API base (e.g. `http://host:7070/v1`); `dimensions` is what the model
    /// produces, carried in config so callers can size the vector store without a probe.
    pub fn new(endpoint: &str, model: impl Into<String>, dimensions: usize) -> OpenAiEmbedder {
        OpenAiEmbedder {
            http: reqwest::Client::new(),
            embeddings_url: format!("{}/embeddings", endpoint.trim_end_matches('/')),
            model: model.into(),
            dimensions,
        }
    }
}

#[async_trait]
impl Embedder for OpenAiEmbedder {
    fn dimensions(&self) -> usize {
        self.dimensions
    }

    async fn embed(&self, inputs: &[String]) -> Result<Vec<Embedding>, ModelError> {
        let request = EmbeddingRequest {
            model: &self.model,
            input: inputs,
        };
        let response = self
            .http
            .post(&self.embeddings_url)
            .json(&request)
            .send()
            .await
            .map_err(backend)?
            .error_for_status()
            .map_err(backend)?
            .json::<EmbeddingResponse>()
            .await
            .map_err(backend)?;
        Ok(response
            .data
            .into_iter()
            .map(|datum| datum.embedding)
            .collect())
    }
}

#[derive(serde::Serialize)]
struct EmbeddingRequest<'a> {
    model: &'a str,
    input: &'a [String],
}

#[derive(serde::Deserialize)]
struct EmbeddingResponse {
    data: Vec<EmbeddingDatum>,
}

#[derive(serde::Deserialize)]
struct EmbeddingDatum {
    embedding: Vec<f32>,
}

fn backend(error: reqwest::Error) -> ModelError {
    ModelError::Backend(error.to_string())
}
