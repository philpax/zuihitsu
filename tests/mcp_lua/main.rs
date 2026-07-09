//! The `mcp.<server>.<tool>{ ... }` projection driven through the session VM, deterministically via the
//! scriptable `FakeMcpHost` (no subprocess, no network). Builds a `Session::with_mcp` over a throwaway
//! in-memory engine and runs Lua scripts through `Session::execute`, asserting the marshalling, the
//! result string-vs-table projection, keyword escaping, and that failures are catchable Lua errors
//! (spec §External I/O via MCP).

#[path = "../common/mod.rs"]
mod common;

use std::{
    collections::{BTreeMap, VecDeque},
    sync::Arc,
    time::Duration,
};

use parking_lot::Mutex;
use zuihitsu::{
    Authority, BlockContext, BlockOutcome, Completion, ContentBlock, ConversationId,
    ConversationLocator, Engine, FakeMcpHost, FakeServer, GenerateRequest, GenerateResponse, Graph,
    InstanceFeatures, ManualClock, McpCatalogue, McpError, McpOutput, McpServerConfig, McpTool,
    MemoryStore, ModelClient, ModelError, ScriptedModel, SeedSelf, Server, Session, Teller,
    TerminalCause, ToolCall, TurnId, TurnOutcome, Usage,
};

/// A tool advertised under `name` (the catalogue entry the escape map is built from).
fn tool(name: &str) -> McpTool {
    McpTool {
        name: name.to_owned(),
        description: format!("the {name} tool"),
        input_schema: serde_json::json!({ "type": "object" }),
    }
}

/// A single-text-block result with no structured content.
fn text(body: &str) -> McpOutput {
    McpOutput {
        content: vec![ContentBlock::Text {
            text: body.to_owned(),
        }],
        structured: None,
    }
}

/// A block-duration budget generous enough that an ordinary fake-backed block never trips it; the
/// firing path has its own test with a deliberately slow server and a short budget.
const TEST_BLOCK_TIMEOUT: Duration = Duration::from_secs(30);

/// Run `script` through a session VM whose `mcp` projection is backed by `host`, projecting each named
/// server. The block runs against a throwaway in-memory engine (the scripts touch MCP, not memory).
async fn run(host: FakeMcpHost, servers: &[&str], script: &str) -> BlockOutcome {
    run_bounded(host, servers, script, TEST_BLOCK_TIMEOUT).await
}

/// [`run`] with an explicit per-block duration budget, so the timeout-firing path can drive a short
/// budget against a slow server.
async fn run_bounded(
    host: FakeMcpHost,
    servers: &[&str],
    script: &str,
    block_timeout: Duration,
) -> BlockOutcome {
    let engine = Engine::new(
        Box::new(MemoryStore::new()),
        Graph::open_in_memory().unwrap(),
        Box::new(ManualClock::new(common::time::EARLY)),
    );
    let configs: BTreeMap<String, McpServerConfig> = servers
        .iter()
        .map(|name| ((*name).to_owned(), McpServerConfig::default()))
        .collect();
    let host = Arc::new(host);
    let catalogue = McpCatalogue::probe(&*host, &configs).await.unwrap();
    let session = Session::with_mcp(
        ConversationId::generate(),
        host,
        catalogue,
        InstanceFeatures::default(),
    );
    session
        .execute(
            &engine,
            &BlockContext {
                teller: Teller::Agent,
                authority: Authority::Platform,
                turn_id: TurnId::generate(),
                block_timeout,
                max_block_attempts: 3,
                max_entry_chars: 10_000,
                present_set: Vec::new(),
                dry_run: false,
            },
            script,
        )
        .await
        .unwrap()
}

mod blocks;
