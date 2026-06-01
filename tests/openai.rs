//! Model-gated embedder test: exercises the real OpenAI-compatible endpoint from `config.toml`.
//! Marked `#[ignore]` so the fast lane never hits the network; run with
//! `cargo test --features openai -- --ignored`. Skips cleanly (passes) if the endpoint is absent
//! or unreachable, so it is safe to run anywhere (spec §Validation: the model-gated lane).

#![cfg(feature = "openai")]

use std::path::Path;

use zuihitsu::{Embedder, EnvConfig, OpenAiEmbedder};

fn configured_embedder() -> Option<OpenAiEmbedder> {
    let config = EnvConfig::load(Path::new("config.toml")).ok()?;
    if config.embedding.endpoint.is_empty() {
        eprintln!("skipping: no embedding endpoint configured in config.toml");
        return None;
    }
    Some(OpenAiEmbedder::new(
        &config.embedding.endpoint,
        config.embedding.model,
        config.embedding.dimensions,
    ))
}

#[tokio::test]
#[ignore = "requires a reachable embedding endpoint (config.toml)"]
async fn embeds_text_with_expected_dimensionality() {
    let Some(embedder) = configured_embedder() else {
        return;
    };

    let inputs = vec!["hello".to_owned(), "hello".to_owned(), "world".to_owned()];
    let vectors = match embedder.embed(&inputs).await {
        Ok(vectors) => vectors,
        Err(error) => {
            eprintln!("skipping: embedding endpoint unreachable: {error}");
            return;
        }
    };

    assert_eq!(vectors.len(), 3);
    assert_eq!(vectors[0].len(), embedder.dimensions());

    // The endpoint is not bit-deterministic (continuous batching), so compare by cosine: the same
    // text embeds near-identically, and distinct text is less similar.
    let same = cosine(&vectors[0], &vectors[1]);
    let distinct = cosine(&vectors[0], &vectors[2]);
    assert!(
        same > 0.999,
        "same text should embed near-identically, got cosine {same}"
    );
    assert!(
        distinct < same,
        "distinct text should be less similar: same={same}, distinct={distinct}"
    );
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let magnitude =
        a.iter().map(|x| x * x).sum::<f32>().sqrt() * b.iter().map(|y| y * y).sum::<f32>().sqrt();
    if magnitude > 0.0 {
        dot / magnitude
    } else {
        0.0
    }
}
