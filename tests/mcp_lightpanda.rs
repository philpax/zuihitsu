//! Live test of the real stdio MCP host against the lightpanda binary in the repo (`mcp/lightpanda`,
//! gitignored). Spawns it, snapshots its catalogue, and drives a real `navigate` + `markdown` against
//! a live site. Model-free but network- and binary-dependent, so it is `#[ignore]` and skips with a
//! clear log line when the binary is absent — the fast lane never spawns a subprocess.

#![cfg(feature = "mcp")]

mod common;

use std::{collections::BTreeMap, path::Path, sync::Arc, time::Duration};

use zuihitsu::{
    Authority, BlockContext, BlockOutcome, ContentBlock, ConversationId, Engine, Graph,
    ManualClock, McpCatalogue, McpHost, McpServerConfig, MemoryStore, Session, StdioHost, Teller,
    TurnId,
};

/// The lightpanda MCP server config: the repo binary run as `lightpanda mcp` over stdio.
fn lightpanda() -> Option<McpServerConfig> {
    if !Path::new("mcp/lightpanda").exists() {
        eprintln!("skipping: mcp/lightpanda is not present");
        return None;
    }
    Some(McpServerConfig {
        command: "mcp/lightpanda".to_owned(),
        args: vec!["mcp".to_owned()],
        ..McpServerConfig::default()
    })
}

/// The joined text of an output's text blocks.
fn text(output: &zuihitsu::McpOutput) -> String {
    output
        .content
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text { text } => Some(text.as_str()),
            ContentBlock::Other(_) => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[tokio::test]
#[ignore = "requires the lightpanda binary at mcp/lightpanda and network access"]
async fn lightpanda_spawns_lists_tools_and_extracts_markdown() {
    let Some(config) = lightpanda() else {
        return;
    };
    let instance = StdioHost
        .spawn("lightpanda", &config)
        .await
        .expect("lightpanda should spawn and complete the handshake");

    // The catalogue is snapshotted at spawn; lightpanda advertises navigate + markdown among ~20 tools.
    let tools: Vec<&str> = instance.tools().iter().map(|t| t.name.as_str()).collect();
    eprintln!("lightpanda tools: {tools:?}");
    assert!(tools.contains(&"navigate"), "tools were {tools:?}");
    assert!(tools.contains(&"markdown"), "tools were {tools:?}");

    // Navigate to philpax.me, then extract its markdown from the loaded page.
    instance
        .call(
            "navigate",
            serde_json::json!({ "url": "https://philpax.me" }),
        )
        .await
        .expect("navigate should succeed");
    let markdown = instance
        .call("markdown", serde_json::json!({}))
        .await
        .expect("markdown should succeed");
    let rendered = text(&markdown);
    eprintln!(
        "markdown ({} chars):\n{}",
        rendered.len(),
        &rendered[..rendered.len().min(400)]
    );
    assert!(
        rendered.to_lowercase().contains("philpax"),
        "extracted markdown should mention philpax: {rendered:?}"
    );

    instance.shutdown().await;
}

#[tokio::test]
#[ignore = "requires the lightpanda binary at mcp/lightpanda and network access"]
async fn the_vm_drives_lightpanda_through_the_mcp_projection() {
    let Some(config) = lightpanda() else {
        return;
    };

    // A session VM with lightpanda projected as `mcp.lightpanda.*`, over a throwaway in-memory engine
    // (the script touches MCP, not memory).
    let engine = Engine::new(
        Box::new(MemoryStore::new()),
        Graph::open_in_memory().unwrap(),
        Box::new(ManualClock::new(common::time::EARLY)),
    );
    let configs = BTreeMap::from([("lightpanda".to_owned(), config)]);
    let host = Arc::new(StdioHost);
    let catalogue = McpCatalogue::probe(&*host, &configs)
        .await
        .expect("probe lightpanda");
    let session = Session::with_mcp(ConversationId::generate(), host, catalogue);

    // Navigate, then extract the loaded page's markdown — both through the Lua projection.
    let outcome = session
        .execute(
            &engine,
            &BlockContext {
                teller: Teller::Agent,
                authority: Authority::Platform,
                turn_id: TurnId::generate(),
                // A live browser fetch is slow; give the block a generous budget so the real network
                // round-trip is never mistaken for a hang.
                block_timeout: Duration::from_secs(60),
                max_block_attempts: 3,
            },
            r#"
            mcp.lightpanda.navigate{ url = "https://philpax.me" }
            return mcp.lightpanda.markdown{}
            "#,
        )
        .await
        .expect("the block runs");
    session.shutdown_mcp().await;

    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected the block to commit, got {outcome:?}");
    };
    eprintln!(
        "markdown via VM ({} chars):\n{}",
        result.len(),
        &result[..result.len().min(400)]
    );
    assert!(
        result.to_lowercase().contains("philpax"),
        "the projected markdown should mention philpax: {result:?}"
    );
}
