//! Reply-lane eval for recurrence emission (spec §Time → TemporalRef, §Validation). Model-gated:
//! `#[ignore]`, skips with a clear log line when no endpoint is reachable from `config.toml`, so the
//! fast lane never hits the network.
//!
//! The deterministic tests already prove a `Recurring` occurrence stores and reads back (the graph
//! occurrence tests); this asks the harder question only the real model answers — does it *emit* a
//! `Recurring` `TemporalRef` for a plainly recurring phrase ("every Tuesday"), rather than flattening
//! it to a single `Day`? A tracked quality rate, not a safety gate: a low rate is a tuning signal for
//! the extraction prompt, so it asserts only a minimal floor.

mod common;

use zuihitsu::{
    ConversationLocator, EnvConfig, Graph, ManualClock, MemoryStore, OpenAiClient, SeedSelf, Server,
};

/// How many times the single-turn scenario is driven; the emission rate is reported over this.
const N: usize = 8;

#[tokio::test]
#[ignore = "requires a reachable model endpoint (config.toml)"]
async fn the_model_emits_a_recurring_occurrence() {
    init_tracing();
    let Some(client) = configured_client() else {
        return;
    };
    let leads = ConversationLocator::new("discord", "leads");

    let mut emitted = 0usize;
    for run in 0..N {
        let server = born_agent();
        if let Err(error) = server
            .platform()
            .route_message(
                &client,
                &leads,
                "dave",
                "Please remember that I have a team standup every Tuesday at 9am.",
                &["dave"],
            )
            .await
        {
            tracing::warn!(%error, "skipping: model became unreachable mid-run");
            return;
        }

        // The model emitted a recurrence iff some memory now carries a `Recurring` occurrence.
        let recurring = server.control().recurring().unwrap();
        if !recurring.is_empty() {
            emitted += 1;
        }
        tracing::info!(
            run,
            recurring = ?recurring.iter().map(|m| m.name.as_str().to_owned()).collect::<Vec<_>>(),
            "recurrence-emission run"
        );
    }

    let rate = emitted as f64 / N as f64;
    tracing::info!(
        emitted,
        total = N,
        rate,
        "recurrence emission rate (tracked, non-gating)"
    );
    // Non-gating: a low rate is a tuning signal for the extraction prompt, not a stop. Assert only a
    // floor, so a wholly broken extraction surfaces as a hard failure rather than a silent zero.
    assert!(
        emitted >= 1,
        "zero recurrence emissions across {N} runs — the temporal extraction may be broken"
    );
}

/// A born agent over an in-memory store, the clock at a present-day, non-epoch time
/// (2026-06-08T00:00:00Z) so the model resolves "every Tuesday" against a lifelike "now".
fn born_agent() -> Server {
    let clock = ManualClock::new(common::time::TEST_NOW);
    let server = Server::new(
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
