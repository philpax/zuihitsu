//! The `mcp.<server>.<tool>{ ... }` projection driven through the session VM, deterministically via the
//! scriptable `FakeMcpHost` (no subprocess, no network). Builds a `Session::with_mcp` over a throwaway
//! in-memory engine and runs Lua scripts through `Session::execute`, asserting the marshalling, the
//! result string-vs-table projection, keyword escaping, and that failures are catchable Lua errors
//! (spec §External I/O via MCP).

mod common;

use std::{collections::BTreeMap, sync::Arc, time::Duration};

use zuihitsu::{
    Authority, BlockContext, BlockOutcome, Completion, ContentBlock, ConversationId,
    ConversationLocator, Engine, FakeMcpHost, FakeServer, Graph, ManualClock, McpCatalogue,
    McpError, McpOutput, McpServerConfig, McpTool, MemoryStore, ScriptedModel, SeedSelf, Server,
    Session, Teller, TerminalCause, ToolCall, TurnId, TurnOutcome,
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
    let session = Session::with_mcp(ConversationId::generate(), host, catalogue);
    session
        .execute(
            &engine,
            &BlockContext {
                teller: Teller::Agent,
                authority: Authority::Platform,
                turn_id: TurnId::generate(),
                block_timeout,
                max_block_attempts: 3,
                present_set: Vec::new(),
                dry_run: false,
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

#[tokio::test(start_paused = true)]
async fn a_block_that_outruns_its_time_budget_is_aborted() {
    // A server whose call hangs far past the block's budget. With the clock paused, the runtime
    // advances to the earliest pending timer — the 1s block timeout fires long before the 120s call
    // would return, so the block aborts and emits nothing but the terminal cause (spec §Concurrency).
    let host = FakeMcpHost::new().with(
        "slow",
        FakeServer::new(vec![tool("crawl")])
            .returns("crawl", text("too late"))
            .with_latency(Duration::from_secs(120)),
    );
    let outcome = run_bounded(
        host,
        &["slow"],
        r#"return mcp.slow.crawl{}"#,
        Duration::from_secs(1),
    )
    .await;
    let BlockOutcome::Terminated(TerminalCause::Error(message)) = outcome else {
        panic!("expected a terminal timeout error, got {outcome:?}");
    };
    assert!(message.contains("time budget"), "message was {message:?}");
    // The block made an MCP call (an un-rollback-able effect), so it is surfaced immediately, NOT
    // retried — the message names no attempt count (spec §645).
    assert!(!message.contains("attempts"), "message was {message:?}");
}

#[tokio::test]
async fn an_uncatalogued_tool_is_nil() {
    let host = FakeMcpHost::new().with(
        "browser",
        FakeServer::new(vec![tool("markdown")]).returns("markdown", text("ok")),
    );
    // The catalogue advertises only `markdown`, so the projection installs only that function — an
    // uncatalogued (or filtered-out) tool simply has no `mcp.browser.*` function and is nil.
    let outcome = run(host, &["browser"], r#"return mcp.browser.nonexistent{}"#).await;
    let BlockOutcome::Terminated(TerminalCause::Error(message)) = outcome else {
        panic!("expected a terminal error, got {outcome:?}");
    };
    assert!(message.contains("nil"), "message was {message:?}");
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

#[tokio::test]
async fn the_catalogue_renders_callable_entries_for_the_prompt() {
    // `goto` is a Lua keyword, so it renders (and is callable) as the escaped `goto_`.
    let host = FakeMcpHost::new().with(
        "browser",
        FakeServer::new(vec![tool("markdown"), tool("goto")]),
    );
    let configs = BTreeMap::from([("browser".to_owned(), McpServerConfig::default())]);
    let host = Arc::new(host);
    let catalogue = McpCatalogue::probe(&*host, &configs).await.unwrap();
    let session = Session::with_mcp(ConversationId::generate(), host, catalogue);
    let calls: Vec<String> = session
        .mcp_api_entries()
        .into_iter()
        .map(|entry| entry.call)
        .collect();
    assert!(
        calls.contains(&"mcp.browser.markdown".to_owned()),
        "{calls:?}"
    );
    assert!(calls.contains(&"mcp.browser.goto_".to_owned()), "{calls:?}");
}

/// A `run_lua` tool call carrying `script`, as the model emits it.
fn run_lua_call(script: &str) -> Completion {
    Completion::ToolCalls(vec![ToolCall {
        id: "1".to_owned(),
        name: "run_lua".to_owned(),
        arguments: serde_json::json!({ "script": script }).to_string(),
    }])
}

#[tokio::test]
async fn the_agent_reaches_an_mcp_tool_through_the_whole_server_path() {
    // A born agent over an in-memory store, with a browser server connected.
    let mut server = Server::new(
        Box::new(MemoryStore::new()),
        Graph::open_in_memory().unwrap(),
        Box::new(ManualClock::new(common::time::TEST_NOW)),
    );
    server
        .control()
        .create_agent(&SeedSelf {
            agent_name: "Kestrel".to_owned(),
            persona: "An assistant.".to_owned(),
            seed_entries: vec![],
        })
        .unwrap();
    let host = FakeMcpHost::new().with(
        "browser",
        FakeServer::new(vec![tool("markdown")]).returns("markdown", text("# Hello from MCP")),
    );
    server
        .connect_mcp(
            Arc::new(host),
            BTreeMap::from([("browser".to_owned(), McpServerConfig::default())]),
        )
        .await
        .unwrap();

    // The scripted model fetches the page through `mcp.browser.markdown` and records it, then replies.
    let model = ScriptedModel::new([
        run_lua_call(r#"memory.create("topic/page", mcp.browser.markdown{})"#),
        Completion::Reply("Saved the page.".to_owned()),
    ]);
    let outcome = server
        .platform()
        .route_message(
            &model,
            &ConversationLocator::new("discord", "general"),
            "phil",
            "save the page",
            &["phil"],
        )
        .await
        .unwrap();
    assert!(
        matches!(outcome, TurnOutcome::Reply(_)),
        "outcome was {outcome:?}"
    );

    // The MCP result reached the block and was written to memory through connect_mcp → ensure_session
    // → with_mcp → the projection → the live call.
    let entries = server.control().entries("topic/page").unwrap();
    assert_eq!(entries.len(), 1);
    assert!(
        entries[0].text.contains("Hello from MCP"),
        "entry was {:?}",
        entries[0].text
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_turn_runs_on_a_worker_thread() {
    // On a multi-thread runtime, `tokio::spawn` requires the spawned future to be `Send`. Driving a
    // whole turn — VM, engine, and MCP projection — inside a spawned task is a compile-time-plus-runtime
    // proof that the turn future is `Send` (it fails to build if a `!Send` capture creeps back in).
    let mut server = Server::new(
        Box::new(MemoryStore::new()),
        Graph::open_in_memory().unwrap(),
        Box::new(ManualClock::new(common::time::TEST_NOW)),
    );
    server
        .control()
        .create_agent(&SeedSelf {
            agent_name: "Kestrel".to_owned(),
            persona: "An assistant.".to_owned(),
            seed_entries: vec![],
        })
        .unwrap();
    let host = FakeMcpHost::new().with(
        "browser",
        FakeServer::new(vec![tool("markdown")]).returns("markdown", text("# Hello from MCP")),
    );
    server
        .connect_mcp(
            Arc::new(host),
            BTreeMap::from([("browser".to_owned(), McpServerConfig::default())]),
        )
        .await
        .unwrap();

    // Share the server behind an `Arc` and drive the turn from a spawned task holding a clone:
    // `route_message` now takes `&self`, so this exercises the shared-server path (not an owned move),
    // and the spawned future must still be `Send`.
    let server = Arc::new(server);
    let outcome = tokio::spawn({
        let server = server.clone();
        async move {
            let model = ScriptedModel::new([
                run_lua_call(r#"memory.create("topic/page", mcp.browser.markdown{})"#),
                Completion::Reply("done".to_owned()),
            ]);
            server
                .platform()
                .route_message(
                    &model,
                    &ConversationLocator::new("discord", "general"),
                    "phil",
                    "save the page",
                    &["phil"],
                )
                .await
        }
    })
    .await
    .expect("the spawned turn task joins")
    .expect("the turn runs");
    assert!(
        matches!(outcome, TurnOutcome::Reply(_)),
        "outcome was {outcome:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_turns_on_distinct_conversations_share_one_server() {
    // Two conversations run at once against a single shared `Arc<Server>`, each writing its own
    // memory. A smoke test that the `&self` facets admit concurrent turns from a shared server (the
    // per-memory locking that makes *same*-memory contention safe lands in the next commit).
    let server = Server::new(
        Box::new(MemoryStore::new()),
        Graph::open_in_memory().unwrap(),
        Box::new(ManualClock::new(common::time::TEST_NOW)),
    );
    server
        .control()
        .create_agent(&SeedSelf {
            agent_name: "Kestrel".to_owned(),
            persona: "An assistant.".to_owned(),
            seed_entries: vec![],
        })
        .unwrap();
    let server = Arc::new(server);

    let turn = |room: &'static str, topic: &'static str, sender: &'static str| {
        let server = server.clone();
        async move {
            let model = ScriptedModel::new([
                run_lua_call(&format!(r#"memory.create("topic/{topic}", "from {room}")"#)),
                Completion::Reply("done".to_owned()),
            ]);
            server
                .platform()
                .route_message(
                    &model,
                    &ConversationLocator::new("discord", room),
                    sender,
                    "note it",
                    &[sender],
                )
                .await
        }
    };

    let (a, b) = tokio::join!(
        tokio::spawn(turn("general", "alpha", "phil")),
        tokio::spawn(turn("random", "beta", "sam")),
    );
    assert!(matches!(
        a.expect("task a joins").expect("turn a runs"),
        TurnOutcome::Reply(_)
    ));
    assert!(matches!(
        b.expect("task b joins").expect("turn b runs"),
        TurnOutcome::Reply(_)
    ));
    // Both turns' writes landed through the shared engine.
    assert!(server.control().memory("topic/alpha").unwrap().is_some());
    assert!(server.control().memory("topic/beta").unwrap().is_some());
}
