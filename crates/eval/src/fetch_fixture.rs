//! The test-only MCP fetch server and its canned content — the fixture a `needs_mcp()` scenario
//! depends on. The server advertises a `markdown` tool that returns a large article (well over the
//! default 1000-char memory entry limit), so an agent that fetches it receives a real block of text
//! and must decide whether to paste it whole (rejected by the limit) or summarize it (accepted).
//!
//! Pure in-memory: the `FakeMcpHost` returns canned results deterministically — no subprocess, no
//! network. Real MCP servers (`config.mcp`) never reach the eval; this host is constructed here and
//! passed via `RunDeps`, separate from the serving host's `connect_mcp(StdioHost, …)`.

use std::sync::Arc;

use zuihitsu::{ContentBlock, FakeMcpHost, FakeServer, McpOutput, McpTool};

/// Build the test fetch host: a single "fetch" server whose `markdown` tool returns a large canned
/// article. Connected per-run in `RunContext::new` before `server.boot()`.
pub fn fetch_host() -> Arc<FakeMcpHost> {
    Arc::new(
        FakeMcpHost::new().with(
            "fetch",
            FakeServer::new(vec![McpTool {
                name: "markdown".to_owned(),
                description: "fetch a page as markdown".to_owned(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": { "url": { "type": "string" } },
                    "required": ["url"]
                }),
            }])
            .returns(
                "markdown",
                McpOutput {
                    content: vec![ContentBlock::Text {
                        text: LARGE_CANNED_ARTICLE.to_owned(),
                    }],
                    structured: None,
                },
            ),
        ),
    )
}

/// A deterministic body of text well over the default 1000-char memory entry limit, returned by the
/// test `fetch` server's `markdown` tool. The agent receives it from `mcp.fetch.markdown{ url = … }`
/// and must decide whether to paste it whole (rejected by the limit) or summarize it (accepted).
const LARGE_CANNED_ARTICLE: &str = "\
The Helix Cascade Protocol: A Field Report
==========================================

The Helix Cascade Protocol was developed in 2024 by Dr. Renata Voss and her team at the Nordic \
Institute for Coastal Research. The protocol describes a method for tracking seasonal nutrient \
flows in fjord ecosystems using a combination of satellite imagery, autonomous underwater \
vehicles, and manual sampling at fixed stations. The core insight is that nutrient pulses in \
fjord waters follow a predictable cascade pattern: deep-water upwelling in early spring triggers \
a phytoplankton bloom, which in turn feeds a zooplankton population surge, which then supports \
the seasonal fish migration. By measuring the timing and intensity of each stage, researchers \
can predict the productivity of the entire food web for the coming season.

The protocol has been adopted by research institutions in Norway, Iceland, and Canada, with \
plans to expand to Chile and New Zealand. The data collected under the protocol is shared \
through an open repository, allowing cross-comparison between fjord systems in different \
hemispheres. Early results suggest that warming waters are compressing the cascade timeline, \
with the spring bloom arriving up to two weeks earlier than it did a decade ago. This \
compression has downstream effects: zooplankton populations may not have time to build before \
the fish arrive, leading to reduced juvenile survival rates in several commercially important \
species.

Dr. Voss has emphasized that the protocol is not a predictive model but a descriptive framework. \
It does not tell you what will happen; it tells you what is happening now, in enough detail that \
you can reason about what might come next. The distinction matters because fjord ecosystems are \
notoriously variable, and a framework that claims prediction in such a system is likely \
overreaching. The protocol's value is in the consistency of its measurements, which allow \
researchers to detect change against a stable baseline rather than against a model that may \
itself be wrong.

The protocol's sampling stations are positioned at the mouth, mid-point, and head of each fjord, \
capturing the gradient from open ocean to enclosed basin. At each station, researchers measure \
temperature, salinity, dissolved oxygen, chlorophyll concentration, and nutrient levels at five \
depths. The autonomous vehicles supplement these point measurements with continuous transects \
along the fjord's length, creating a high-resolution picture of the water column. Satellite \
imagery provides the broadest view, tracking surface temperature and chlorophyll across the \
entire fjord system. The three data streams are integrated weekly during the active season and \
monthly during the winter quiet period.

Funding for the protocol comes from a mix of national research councils, the European Union's \
marine science framework, and several private foundations focused on ocean conservation. The \
total annual budget is approximately 4.2 million euros, split roughly equally between the \
participating institutions. The protocol's open-data policy has been a condition of funding from \
the start, and all data is published within six months of collection.";
