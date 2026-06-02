//! Model-gated embedder test: exercises the real OpenAI-compatible endpoint from `config.toml`.
//! Marked `#[ignore]` so the fast lane never hits the network; run with
//! `cargo test --features openai -- --ignored`. Skips cleanly (passes) if the endpoint is absent
//! or unreachable, so it is safe to run anywhere (spec §Validation: the model-gated lane).

#![cfg(feature = "openai")]

use std::path::Path;

use zuihitsu::{
    Completion, Embedder, EnvConfig, GenerateRequest, Message, ModelClient, OpenAiClient,
    OpenAiEmbedder, ToolChoice, ToolSpec,
};

fn configured_client() -> Option<OpenAiClient> {
    let config = EnvConfig::load(Path::new("config.toml")).ok()?;
    if config.model.endpoint.is_empty() {
        tracing::warn!("skipping: no model endpoint configured in config.toml");
        return None;
    }
    Some(OpenAiClient::new(&config.model))
}

/// Install a test-scoped tracing subscriber so these model-gated tests emit structured, timestamped
/// logs (visible under `--nocapture`) rather than ad-hoc prints. Idempotent across tests in the
/// binary.
fn init_tracing() {
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_max_level(tracing::Level::DEBUG)
        .try_init();
}

#[tokio::test]
#[ignore = "requires a reachable model endpoint (config.toml)"]
async fn generates_a_reply() {
    init_tracing();
    let Some(client) = configured_client() else {
        return;
    };
    let request = GenerateRequest {
        system: "You are concise. Answer in one short sentence.".to_owned(),
        messages: vec![Message::user("Say hello.")],
        tools: Vec::new(),
        tool_choice: ToolChoice::Auto,
        thinking: None,
    };
    match client.generate(&request).await {
        Ok(Completion::Reply(text)) => {
            assert!(!text.trim().is_empty(), "reply should be non-empty")
        }
        Ok(other) => panic!("expected a reply, got {other:?}"),
        Err(error) => tracing::warn!(%error, "skipping: model unreachable"),
    }
}

#[tokio::test]
#[ignore = "requires a reachable model endpoint (config.toml)"]
async fn emits_a_run_lua_tool_call() {
    init_tracing();
    let Some(client) = configured_client() else {
        return;
    };
    let request = GenerateRequest {
        system: "You act only by emitting Lua through the run_lua tool. To remember something, \
                 call run_lua with a script that calls memory.create(name, text)."
            .to_owned(),
        messages: vec![Message::user(
            "Please remember that Dave climbs at the bouldering gym.",
        )],
        tools: vec![ToolSpec {
            name: "run_lua".to_owned(),
            description: "Execute a Lua block against your memory.".to_owned(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": { "script": { "type": "string" } },
                "required": ["script"]
            }),
        }],
        tool_choice: ToolChoice::Auto,
        thinking: None,
    };
    match client.generate(&request).await {
        Ok(Completion::ToolCalls(calls)) => {
            assert_eq!(calls[0].name, "run_lua");
            let args: serde_json::Value =
                serde_json::from_str(&calls[0].arguments).expect("tool arguments are JSON");
            assert!(
                args.get("script").and_then(|s| s.as_str()).is_some(),
                "the call carries a `script` string, got {args}"
            );
        }
        Ok(other) => panic!("expected a run_lua tool call, got {other:?}"),
        Err(error) => tracing::warn!(%error, "skipping: model unreachable"),
    }
}

fn configured_embedder() -> Option<OpenAiEmbedder> {
    let config = EnvConfig::load(Path::new("config.toml")).ok()?;
    if config.embedding.endpoint.is_empty() {
        tracing::warn!("skipping: no embedding endpoint configured in config.toml");
        return None;
    }
    Some(OpenAiEmbedder::new(&config.embedding))
}

#[tokio::test]
#[ignore = "requires a reachable embedding endpoint (config.toml)"]
async fn embeds_text_with_expected_dimensionality() {
    init_tracing();
    let Some(embedder) = configured_embedder() else {
        return;
    };

    let inputs = vec!["hello".to_owned(), "hello".to_owned(), "world".to_owned()];
    let vectors = match embedder.embed(&inputs).await {
        Ok(vectors) => vectors,
        Err(error) => {
            tracing::warn!(%error, "skipping: embedding endpoint unreachable");
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
