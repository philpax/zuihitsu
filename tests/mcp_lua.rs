//! The `mcp.<server>.<tool>{ ... }` projection driven through the session VM, deterministically via the
//! scriptable `FakeMcpHost` (no subprocess, no network). Builds a `Session::with_mcp` over a throwaway
//! in-memory engine and runs Lua scripts through `Session::execute`, asserting the marshalling, the
//! result string-vs-table projection, keyword escaping, and that failures are catchable Lua errors
//! (spec §External I/O via MCP).

#![cfg(all(feature = "lua", feature = "mcp"))]

use std::{collections::BTreeMap, rc::Rc};

use zuihitsu::{
    Authority, BlockContext, BlockOutcome, ContentBlock, ConversationId, Engine, FakeMcpHost,
    FakeServer, Graph, ManualClock, McpError, McpOutput, McpServerConfig, McpTool, MemoryStore,
    Session, Teller, TerminalCause, Timestamp, TurnId,
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

/// Run `script` through a session VM whose `mcp` projection is backed by `host`, projecting each named
/// server. The block runs against a throwaway in-memory engine (the scripts touch MCP, not memory).
async fn run(host: FakeMcpHost, servers: &[&str], script: &str) -> BlockOutcome {
    let engine = Engine::new(
        Box::new(MemoryStore::new()),
        Graph::open_in_memory().unwrap(),
        Box::new(ManualClock::new(Timestamp::from_millis(1_000))),
    );
    let servers: BTreeMap<String, McpServerConfig> = servers
        .iter()
        .map(|name| ((*name).to_owned(), McpServerConfig::default()))
        .collect();
    let session = Session::with_mcp(ConversationId::generate(), Rc::new(host), servers);
    session
        .execute(
            &engine,
            &BlockContext {
                teller: Teller::Agent,
                authority: Authority::Platform,
                turn_id: TurnId::generate(),
            },
            script,
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn an_all_text_result_returns_a_bare_string() {
    let host = FakeMcpHost::new().with(
        "browser",
        FakeServer::new(vec![tool("markdown")]).returns("markdown", text("# Hello")),
    );
    let outcome = run(host, &["browser"], r#"return mcp.browser.markdown{}"#).await;
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit, got {outcome:?}");
    };
    // The common case: a single text block projects to a bare Lua string.
    assert_eq!(result, "# Hello");
}

#[tokio::test]
async fn structured_content_returns_a_table() {
    let host = FakeMcpHost::new().with(
        "browser",
        FakeServer::new(vec![tool("query")]).returns(
            "query",
            McpOutput {
                content: vec![ContentBlock::Text {
                    text: "ignored".to_owned(),
                }],
                structured: Some(serde_json::json!({ "count": 3 })),
            },
        ),
    );
    // structuredContent forces the table shape; read it back through `.structured`.
    let outcome = run(
        host,
        &["browser"],
        r#"return mcp.browser.query{}.structured.count"#,
    )
    .await;
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit, got {outcome:?}");
    };
    assert_eq!(result, "3");
}

#[tokio::test]
async fn a_non_text_block_returns_a_table_of_blocks() {
    let host = FakeMcpHost::new().with(
        "browser",
        FakeServer::new(vec![tool("links")]).returns(
            "links",
            McpOutput {
                content: vec![ContentBlock::Other(serde_json::json!({
                    "type": "resource",
                    "uri": "https://example.com",
                }))],
                structured: None,
            },
        ),
    );
    let outcome = run(
        host,
        &["browser"],
        r#"return mcp.browser.links{}.content[1].type"#,
    )
    .await;
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit, got {outcome:?}");
    };
    assert_eq!(result, "resource");
}

#[tokio::test]
async fn a_keyword_tool_is_callable_escaped() {
    // lightpanda advertises `goto`; it is callable as the keyword-escaped `goto_` and routes to raw `goto`.
    let host = FakeMcpHost::new().with(
        "browser",
        FakeServer::new(vec![tool("goto")]).returns("goto", text("navigated")),
    );
    let outcome = run(
        host,
        &["browser"],
        r#"return mcp.browser.goto_{ url = "https://example.com" }"#,
    )
    .await;
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit, got {outcome:?}");
    };
    assert_eq!(result, "navigated");
}

#[tokio::test]
async fn a_tool_error_is_a_catchable_lua_error() {
    let host = FakeMcpHost::new().with(
        "browser",
        FakeServer::new(vec![tool("boom")]).fails("boom", McpError::Tool("kaboom".to_owned())),
    );
    // pcall catches it, so the block still commits — the agent can adapt.
    let outcome = run(
        host,
        &["browser"],
        r#"
        local ok, err = pcall(function() return mcp.browser.boom{} end)
        return tostring(ok) .. ": " .. tostring(err)
        "#,
    )
    .await;
    let BlockOutcome::Committed { result } = outcome else {
        panic!("expected commit, got {outcome:?}");
    };
    assert!(result.starts_with("false: "), "result was {result:?}");
    assert!(result.contains("mcp:"), "result was {result:?}");
    assert!(result.contains("kaboom"), "result was {result:?}");
}

#[tokio::test]
async fn an_uncaught_tool_error_terminates_the_block() {
    let host = FakeMcpHost::new().with(
        "browser",
        FakeServer::new(vec![tool("boom")]).fails("boom", McpError::Tool("kaboom".to_owned())),
    );
    let outcome = run(host, &["browser"], r#"return mcp.browser.boom{}"#).await;
    let BlockOutcome::Terminated(TerminalCause::Error(message)) = outcome else {
        panic!("expected a terminal error, got {outcome:?}");
    };
    assert!(message.contains("kaboom"), "message was {message:?}");
}

#[tokio::test]
async fn an_unknown_tool_errors() {
    let host = FakeMcpHost::new().with(
        "browser",
        FakeServer::new(vec![tool("markdown")]).returns("markdown", text("ok")),
    );
    // The server spawns and lists `markdown`; `nonexistent` resolves to no raw tool.
    let outcome = run(host, &["browser"], r#"return mcp.browser.nonexistent{}"#).await;
    let BlockOutcome::Terminated(TerminalCause::Error(message)) = outcome else {
        panic!("expected a terminal error, got {outcome:?}");
    };
    assert!(message.contains("no tool"), "message was {message:?}");
}

#[tokio::test]
async fn an_unconfigured_server_is_nil() {
    let host = FakeMcpHost::new().with(
        "browser",
        FakeServer::new(vec![tool("markdown")]).returns("markdown", text("ok")),
    );
    // `mcp.other` was never configured, so it is nil — indexing a tool on it is a plain Lua error.
    let outcome = run(host, &["browser"], r#"return mcp.other.foo{}"#).await;
    assert!(
        matches!(outcome, BlockOutcome::Terminated(TerminalCause::Error(_))),
        "expected a terminal error, got {outcome:?}"
    );
}
