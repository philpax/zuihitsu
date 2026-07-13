//! The test-only web-fetch fixture: a [`FakeWebFetcher`] serving canned pages that a fetching
//! scenario reads through `web.markdown(url)`. Pure in-memory — no subprocess, no network — so a run
//! that fetches is deterministic. The real [`HttpFetcher`](zuihitsu::HttpFetcher) never reaches the
//! eval; this fetcher is constructed here and wired in per run via `RunDeps`, standing in for what
//! the serving host builds from config.
//!
//! Two pages are served. The first is a chrome-heavy article well over the default memory entry
//! limit (the fixture the content-limit scenario depends on), so an agent that fetches it holds a
//! real block of text and must summarize rather than paste. The second is a chrome-heavy page for an
//! invented open-source project, so a browsing scenario can check that what the agent stores carries
//! the README's prose, not the page's navigation and sidebar chrome.

use std::sync::Arc;

use zuihitsu::FakeWebFetcher;

/// The URL the content-limit scenario fetches — the large article below.
pub const ARTICLE_URL: &str = "https://example.com/helix-cascade";

/// The URL the browsing scenario fetches — the invented project page below.
pub const PROJECT_URL: &str = "https://forge.example/quill-labs/tessera";

/// The Markdown character cap the fixture fetcher applies — the production default, so truncation
/// behaves as it would live.
pub const FIXTURE_MAX_MARKDOWN_CHARS: usize = 20_000;

/// Build the fixture fetcher, serving both canned pages. Connected per run in `assemble` before
/// `server.boot()`.
pub fn web_fetcher() -> Arc<FakeWebFetcher> {
    Arc::new(
        FakeWebFetcher::new()
            .with_html(ARTICLE_URL, ARTICLE_HTML)
            .with_html(PROJECT_URL, PROJECT_HTML),
    )
}

/// A chrome-heavy article page whose extracted main content runs well over the default 1000-char
/// memory entry limit. The agent receives it from `web.markdown(...)` and must decide whether to
/// paste it whole (rejected by the limit) or summarize it (accepted). The surrounding nav, cookie
/// banner, and footer are the chrome extraction strips.
const ARTICLE_HTML: &str = include_str!("fixtures/helix_article.html");

/// A chrome-heavy project page for an invented open-source library, in the shape of a code-forge repo
/// view: a top navigation bar, sign-in prompts, star and fork counts, a file listing, and a sidebar
/// of repository metadata, around a substantive README. A browsing scenario checks that the agent's
/// stored memory reflects the README's prose, not this chrome.
const PROJECT_HTML: &str = include_str!("fixtures/tessera_project.html");
