//! The `web.markdown(url)` projection driven through the session VM, deterministically via the
//! scriptable `FakeWebFetcher` (no network). Builds a `Session` over a throwaway in-memory engine and
//! runs Lua scripts through `Session::execute`, asserting the pipeline output, each teachable failure
//! mode, and that the module is absent (a nil-call error) when the `browsing` feature is off.

#[path = "common/mod.rs"]
mod common;

use std::{sync::Arc, time::Duration};

use zuihitsu::{
    Authority, BlockContext, BlockOutcome, Completion, ConversationId, ConversationLocator, Engine,
    FakeWebFetcher, FetchedPage, Graph, InstanceFeatures, ManualClock, MemoryStore, PersonId,
    ScriptedModel, SeedSelf, Server, Session, TEST_PLATFORM, Teller, TerminalCause, ToolCall,
    TurnId, TurnOutcome, WebClient, WebError,
};

/// The Markdown character cap for these tests — generous enough that the fixture is never truncated.
const TEST_MAX_MARKDOWN_CHARS: usize = 20_000;

/// A chrome-heavy article page: a nav header, a cookie banner, a sidebar, the real article body, and
/// a footer. Extraction should keep the prose and drop the furniture.
const CHROME_HEAVY_HTML: &str = "<!doctype html><html><head>\
    <title>The Meridian Compiler — Overview</title></head><body>\
    <div class=\"cookie-banner\">We use cookies. Accept all cookies to continue.</div>\
    <header><nav><a href=\"/\">Home</a><a href=\"/docs\">Docs</a><a href=\"/login\">Sign in</a></nav>\
    </header>\
    <aside><ul><li><a href=\"/promo\">Sponsored: our newsletter</a></li></ul></aside>\
    <main><article><h1>The Meridian Compiler — Overview</h1>\
    <p>Meridian is an ahead-of-time compiler for a small statically-typed language. It lowers the \
    source through a typed intermediate representation before emitting native code, so type errors \
    are caught long before code generation begins.</p>\
    <p>Its optimizer runs a fixed set of passes — constant folding, dead-code elimination, and \
    inlining — chosen so that the same input always produces the same output, which keeps builds \
    reproducible and easy to cache.</p>\
    <p>The toolchain ships a language server that reuses the compiler front end, so editor \
    diagnostics match what a full build would report exactly.</p>\
    </article></main>\
    <footer>© 2026 Meridian. Terms of service · Privacy policy</footer></body></html>";

/// Run `script` through a session VM whose `web` projection is backed by `fetcher`, under `features`.
async fn run(fetcher: FakeWebFetcher, features: InstanceFeatures, script: &str) -> BlockOutcome {
    let engine = Engine::new(
        Box::new(MemoryStore::new()),
        Graph::open_in_memory().unwrap(),
        Box::new(ManualClock::new(common::time::EARLY)),
    );
    let session = Session::new(Some(ConversationId::generate()), features).with_web(Some(
        WebClient::new(Arc::new(fetcher), TEST_MAX_MARKDOWN_CHARS),
    ));
    session
        .execute(
            &engine,
            &BlockContext {
                teller: Teller::Agent,
                authority: Authority::Platform,
                turn_id: TurnId::generate(),
                block_timeout: Duration::from_secs(30),
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

fn committed(outcome: BlockOutcome) -> String {
    match outcome {
        BlockOutcome::Committed { result } => result,
        other => panic!("expected a commit, got {other:?}"),
    }
}

fn terminal_error(outcome: BlockOutcome) -> String {
    match outcome {
        BlockOutcome::Terminated(TerminalCause::Error(message)) => message,
        other => panic!("expected a terminal error, got {other:?}"),
    }
}

#[tokio::test]
async fn markdown_returns_the_extracted_page_without_chrome() {
    let url = "https://example.com/meridian";
    let fetcher = FakeWebFetcher::new().with_html(url, CHROME_HEAVY_HTML);
    let result = committed(
        run(
            fetcher,
            InstanceFeatures::default(),
            &format!("return web.markdown(\"{url}\")"),
        )
        .await,
    );
    assert!(
        result.starts_with("# The Meridian Compiler — Overview"),
        "{result}"
    );
    assert!(result.contains("ahead-of-time compiler"), "{result}");
    // The chrome is gone.
    for chrome in ["We use cookies", "Sign in", "Sponsored", "Privacy policy"] {
        assert!(
            !result.contains(chrome),
            "chrome leaked: {chrome:?}\n{result}"
        );
    }
}

#[tokio::test]
async fn a_non_html_page_surfaces_a_teachable_error() {
    let url = "https://example.com/data.json";
    let fetcher = FakeWebFetcher::new().with_page(
        url,
        FetchedPage {
            final_url: url.to_owned(),
            content_type: "application/json".to_owned(),
            body: "{}".to_owned(),
        },
    );
    let message = terminal_error(
        run(
            fetcher,
            InstanceFeatures::default(),
            &format!("return web.markdown(\"{url}\")"),
        )
        .await,
    );
    assert!(message.contains("not an HTML page"), "{message}");
}

#[tokio::test]
async fn each_scripted_failure_mode_surfaces_its_teachable_error() {
    let cases = [
        (
            WebError::BlockedAddress {
                url: "http://127.0.0.1/".to_owned(),
            },
            "private or loopback",
        ),
        (
            WebError::Status {
                url: "https://example.com/missing".to_owned(),
                status: 404,
            },
            "HTTP status 404",
        ),
        (
            WebError::Timeout {
                url: "https://slow.example/".to_owned(),
            },
            "timed out",
        ),
        (
            WebError::TooLarge {
                url: "https://big.example/".to_owned(),
                limit: 5_000_000,
            },
            "larger than",
        ),
    ];
    for (error, needle) in cases {
        let url = "https://example.com/probe";
        let fetcher = FakeWebFetcher::new().with_error(url, error);
        let message = terminal_error(
            run(
                fetcher,
                InstanceFeatures::default(),
                &format!("return web.markdown(\"{url}\")"),
            )
            .await,
        );
        assert!(
            message.contains(needle) && message.contains("web:"),
            "expected {needle:?} in a web-prefixed error, got {message}"
        );
    }
}

#[tokio::test]
async fn a_non_string_url_argument_is_a_teachable_error() {
    let fetcher = FakeWebFetcher::new();
    let message = terminal_error(
        run(
            fetcher,
            InstanceFeatures::default(),
            "return web.markdown({ url = \"https://example.com\" })",
        )
        .await,
    );
    assert!(message.contains("takes a URL string"), "{message}");
}

#[tokio::test]
async fn a_failed_fetch_is_catchable_so_the_block_commits() {
    let url = "https://example.com/gone";
    let fetcher = FakeWebFetcher::new().with_error(
        url,
        WebError::Status {
            url: url.to_owned(),
            status: 500,
        },
    );
    // pcall catches the fetch error, so the block still commits — the agent can adapt.
    let result = committed(
        run(
            fetcher,
            InstanceFeatures::default(),
            &format!(
                "local ok, err = pcall(function() return web.markdown(\"{url}\") end)\n\
                 return tostring(ok) .. \": \" .. tostring(err)"
            ),
        )
        .await,
    );
    assert!(result.starts_with("false: "), "{result}");
    assert!(result.contains("HTTP status 500"), "{result}");
}

#[tokio::test]
async fn the_agent_reaches_web_markdown_through_the_whole_server_path() {
    // A born agent over an in-memory store, with the fixture fetcher connected via `connect_web`.
    // This exercises the instance-level wiring — `connect_web` → `mint_vm().with_web(...)` — that the
    // Session-level tests above bypass, so the threading that a live turn depends on is proven here
    // rather than only in a GPU eval.
    let url = "https://example.com/meridian";
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
    server.connect_web(
        Arc::new(FakeWebFetcher::new().with_html(url, CHROME_HEAVY_HTML)),
        TEST_MAX_MARKDOWN_CHARS,
    );

    // The scripted model fetches the page through `web.markdown` and records it, then replies.
    let model = ScriptedModel::new([
        Completion::ToolCalls(vec![ToolCall {
            id: "1".to_owned(),
            name: "run_lua".to_owned(),
            arguments: serde_json::json!({
                "script": format!("memory.create(\"topic/page\", web.markdown(\"{url}\"))"),
            })
            .to_string(),
        }]),
        Completion::Reply("Saved the page.".to_owned()),
    ]);
    let outcome = server
        .platform()
        .route_message(
            &model,
            &ConversationLocator::new(TEST_PLATFORM, "general"),
            &PersonId::new(TEST_PLATFORM, "marcus"),
            "save the page",
            &[PersonId::new(TEST_PLATFORM, "marcus")],
        )
        .await
        .unwrap();
    assert!(
        matches!(outcome.outcome, TurnOutcome::Reply(_)),
        "{outcome:?}"
    );

    // The extracted content reached the block and was written to memory.
    let entries = server.control().entries("topic/page").unwrap();
    assert_eq!(entries.len(), 1);
    assert!(
        entries[0].text.contains("ahead-of-time compiler"),
        "entry was {:?}",
        entries[0].text
    );
}

#[tokio::test]
async fn browsing_off_makes_web_a_nil_call() {
    let url = "https://example.com/meridian";
    let fetcher = FakeWebFetcher::new().with_html(url, CHROME_HEAVY_HTML);
    let features = InstanceFeatures {
        browsing: false,
        ..Default::default()
    };
    // With browsing off the `web` global is never installed, so calling through it is the standard
    // Lua nil error — a teachable failure, not a silent no-op.
    let message = terminal_error(
        run(
            fetcher,
            features,
            &format!("return web.markdown(\"{url}\")"),
        )
        .await,
    );
    assert!(message.contains("nil"), "{message}");
}
