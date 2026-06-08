//! Live test of the real stdio MCP host against the lightpanda binary in the repo (`mcp/lightpanda`,
//! gitignored). Spawns it, snapshots its catalogue, and drives a real `navigate` + `markdown` against
//! a live site. Model-free but network- and binary-dependent, so it is `#[ignore]` and skips with a
//! clear log line when the binary is absent — the fast lane never spawns a subprocess.

#![cfg(feature = "mcp")]

use std::path::Path;

use zuihitsu::{ContentBlock, McpHost, McpServerConfig, StdioHost};

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
