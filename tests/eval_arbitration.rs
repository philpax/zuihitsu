//! Reply-lane eval for belief arbitration (spec §Write path, §Validation). Model-gated: `#[ignore]`,
//! skips with a clear log line when no endpoint is reachable from `config.toml`, so the fast lane
//! never hits the network.
//!
//! The deterministic tests prove the mechanism — given an `arbitration` in the `synthesize` call, a
//! `BeliefArbitrated` is recorded. This asks the harder question only the real model answers: shown two
//! directly contradicting statements, does it *flag the conflict* rather than silently smoothing it into
//! a description? A tracked quality rate, not a safety gate (conflict detection is a model judgment), so
//! it asserts only a minimal floor.

#![cfg(all(feature = "lua", feature = "openai"))]

mod common;

use zuihitsu::{
    ConversationLocator, EnvConfig, Graph, ManualClock, MemoryStore, OpenAiClient, SeedSelf, Server,
};

/// How many times the single-turn scenario is driven; the arbitration rate is reported over this.
const N: usize = 8;

#[tokio::test]
#[ignore = "requires a reachable model endpoint (config.toml)"]
async fn the_model_flags_a_direct_contradiction() {
    init_tracing();
    let Some(client) = configured_client() else {
        return;
    };
    let leads = ConversationLocator::new("discord", "leads");

    let mut arbitrated = 0usize;
    for run in 0..N {
        let mut server = born_agent();
        if let Err(error) = server
            .platform()
            .route_message(
                &client,
                &leads,
                "dave",
                "Two colleagues gave me contradictory accounts of Dave: Alice says Dave is a \
                 committed vegetarian who never eats meat, while Bob insists Dave eats steak every \
                 week and isn't vegetarian at all.",
                &["dave"],
            )
            .await
        {
            tracing::warn!(%error, "skipping: model became unreachable mid-run");
            return;
        }

        let arbitrations = server.control().arbitrations().unwrap();
        if !arbitrations.is_empty() {
            arbitrated += 1;
        }
        tracing::info!(
            run,
            arbitrations = ?arbitrations
                .iter()
                .map(|a| format!("{}: {}", a.memory.as_str(), a.statement))
                .collect::<Vec<_>>(),
            "arbitration run"
        );
    }

    let rate = arbitrated as f64 / N as f64;
    tracing::info!(
        arbitrated,
        total = N,
        rate,
        "belief arbitration rate (tracked, non-gating)"
    );
    // Non-gating: a low rate is a tuning signal for the regen prompt, not a stop. Assert only a floor,
    // so a wholly broken path surfaces as a hard failure rather than a silent zero.
    assert!(
        arbitrated >= 1,
        "zero belief arbitrations across {N} runs over a direct contradiction — the path may be broken"
    );
}

/// A born agent over an in-memory store, the clock at a present-day, non-epoch time
/// (2026-06-08T00:00:00Z).
fn born_agent() -> Server {
    let clock = ManualClock::new(common::time::TEST_NOW);
    let mut server = Server::new(
        Box::new(MemoryStore::new()),
        Graph::open_in_memory().unwrap(),
        Box::new(clock),
    );
    server.control().create_agent(&seed()).unwrap();
    server
}

fn seed() -> SeedSelf {
    SeedSelf {
        agent_name: "Kestrel".to_owned(),
        persona: "A general-purpose assistant with a long memory.".to_owned(),
        seed_entries: vec![],
    }
}

fn configured_client() -> Option<OpenAiClient> {
    let config = EnvConfig::load(std::path::Path::new("config.toml")).ok()?;
    if config.model.endpoint.is_empty() {
        tracing::warn!("skipping: no model endpoint configured in config.toml");
        return None;
    }
    Some(OpenAiClient::new(&config.model))
}

/// A test-scoped subscriber so the model-gated run emits structured logs under `--nocapture`.
/// Idempotent across the binary.
fn init_tracing() {
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_max_level(tracing::Level::DEBUG)
        .try_init();
}
