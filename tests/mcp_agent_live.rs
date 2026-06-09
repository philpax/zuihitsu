//! The capstone end-to-end MCP test: a real model, told about the lightpanda browser in its system
//! prompt, *chooses* to call it and reflects the page back. Exercises the whole increment-3 path —
//! config → `connect_mcp` probe → the prompt catalogue → the agent's choice → `ensure_session` →
//! `with_mcp` → the live call. Model- and binary-gated (`#[ignore]`), skips with a clear log line when
//! the endpoint or the lightpanda binary is absent, so the fast lane never hits the network.

#![cfg(all(feature = "lua", feature = "mcp", feature = "openai"))]

use std::{collections::BTreeMap, path::Path, rc::Rc};

use zuihitsu::{
    ConversationLocator, EnvConfig, Graph, ManualClock, McpServerConfig, MemoryStore, OpenAiClient,
    SeedSelf, Server, StdioHost, Timestamp, TurnOutcome,
};

#[tokio::test]
#[ignore = "requires a reachable model endpoint (config.toml) and the lightpanda binary"]
async fn the_agent_chooses_to_browse_with_lightpanda() {
    init_tracing();
    let Some(client) = configured_client() else {
        return;
    };
    if !Path::new("mcp/lightpanda").exists() {
        eprintln!("skipping: mcp/lightpanda is not present");
        return;
    }

    // A born agent with lightpanda connected — so its tools are probed and rendered into the prompt.
    let mut server = born_agent();
    server
        .connect_mcp(
            Rc::new(StdioHost),
            BTreeMap::from([(
                "lightpanda".to_owned(),
                McpServerConfig {
                    command: "mcp/lightpanda".to_owned(),
                    args: vec!["mcp".to_owned()],
                    ..McpServerConfig::default()
                },
            )]),
        )
        .await
        .expect("connect lightpanda");

    let outcome = server
        .platform()
        .route_message(
            &client,
            &ConversationLocator::new("discord", "general"),
            "phil",
            "Use your browser to open https://philpax.me and tell me, in a sentence, what the site is.",
            &["phil"],
        )
        .await
        .expect("the turn runs");

    let TurnOutcome::Reply(reply) = outcome else {
        panic!("expected a reply, got {outcome:?}");
    };
    eprintln!("agent reply:\n{reply}");
    // The site's name only reaches the reply if the agent actually browsed it through the projection.
    assert!(
        reply.to_lowercase().contains("philpax"),
        "the agent's reply should reflect the browsed page: {reply:?}"
    );
}

/// A born agent over an in-memory store, the clock at a present-day, non-epoch time.
fn born_agent() -> Server {
    let clock = ManualClock::new(Timestamp::from_millis(1_780_876_800_000));
    let mut server = Server::new(
        Box::new(MemoryStore::new()),
        Graph::open_in_memory().unwrap(),
        Box::new(clock),
    );
    server
        .control()
        .create_agent(&SeedSelf {
            agent_name: "Kestrel".to_owned(),
            persona: "A general-purpose assistant with a browser.".to_owned(),
            seed_entries: vec![],
        })
        .unwrap();
    server
}

fn configured_client() -> Option<OpenAiClient> {
    let config = EnvConfig::load(Path::new("config.toml")).ok()?;
    if config.model.endpoint.is_empty() {
        tracing::warn!("skipping: no model endpoint configured in config.toml");
        return None;
    }
    Some(OpenAiClient::new(&config.model))
}

fn init_tracing() {
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_max_level(tracing::Level::DEBUG)
        .try_init();
}
