//! In-house web fetching: the agent's first-class reach onto the open web, exposed to a block as
//! `web.markdown(url)`.
//!
//! A fetch is a pipeline: pull the page over HTTP, extract its main content (dropping nav, footers,
//! cookie banners, and the rest of the chrome), render that to Markdown, and cap the result so a long
//! page cannot flood the agent's context. The transport is the one seam ([`WebFetcher`]): the real
//! [`HttpFetcher`] drives reqwest with a per-fetch timeout, a streaming byte cap, and a server-side
//! request forgery guard, while the scriptable [`FakeWebFetcher`] returns canned pages with no
//! network. Everything above the seam — the readability extraction, the Markdown conversion, and the
//! truncation — is the pure [`pipeline`], shared by both, so a fake exercises the same extraction the
//! real path runs.
//!
//! Fetching is a distinct capability from MCP (`crate::mcp`), which remains the seam for
//! operator-configured tools. A GET is idempotent, so — unlike an MCP call — a fetch does not latch
//! the block's "made an external call" flag, and a timed-out fetch-only block stays retryable.

mod fake;
mod http;
mod pipeline;

pub use fake::FakeWebFetcher;
pub use http::{HttpFetcher, HttpFetcherConfig};

use std::sync::Arc;

use async_trait::async_trait;

/// The HTTP transport seam: fetch `url` and hand back the fetched page, or a [`WebError`] the agent
/// can learn from. `Send + Sync` so the fetcher rides behind an `Arc` shared across a multi-thread
/// turn's worker threads, like the MCP host.
#[async_trait]
pub trait WebFetcher: Send + Sync {
    /// GET `url`, following redirects, and return the fetched page — its final (post-redirect) URL,
    /// its content type, and its decoded body.
    async fn fetch(&self, url: &str) -> Result<FetchedPage, WebError>;
}

/// A fetched page, before extraction: the final URL after any redirects (the base the extractor
/// resolves relative links against), the response content type as the server sent it (parameters
/// like `; charset=…` included — [`is_html`] normalises before matching), and the decoded body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FetchedPage {
    pub final_url: String,
    pub content_type: String,
    pub body: String,
}

/// The web fetcher paired with the Markdown character cap — the whole `web.markdown` pipeline behind
/// one handle. Held by the instance and threaded into each session's block API, where the Lua `web`
/// module calls [`WebClient::markdown`]. `Clone` clones the inner `Arc`.
#[derive(Clone)]
pub struct WebClient {
    fetcher: Arc<dyn WebFetcher>,
    max_markdown_chars: usize,
}

impl WebClient {
    pub fn new(fetcher: Arc<dyn WebFetcher>, max_markdown_chars: usize) -> WebClient {
        WebClient {
            fetcher,
            max_markdown_chars,
        }
    }

    /// Fetch `url` and return its main content as Markdown: the transport runs the fetch, then the
    /// pure pipeline extracts the article, renders it, and truncates it to the character cap.
    pub async fn markdown(&self, url: &str) -> Result<String, WebError> {
        let page = self.fetcher.fetch(url).await?;
        pipeline::to_markdown(&page, self.max_markdown_chars)
    }
}

/// A catchable web-fetch failure. Every variant carries the offending URL, and `Display` leads with a
/// `web:` context prefix, per the error convention; the wording is teachable — the agent reads it and
/// adapts, the way it reads a bad-argument error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WebError {
    /// The URL could not be parsed.
    InvalidUrl { url: String, reason: String },
    /// The URL uses a scheme the fetcher does not speak (only `http` and `https` are fetched).
    UnsupportedScheme { url: String, scheme: String },
    /// The URL resolves to a loopback, private, link-local, or unique-local address, and private
    /// fetches are not permitted — the server-side request forgery guard.
    BlockedAddress { url: String },
    /// The fetch exceeded its per-fetch time budget.
    Timeout { url: String },
    /// The server answered with a non-success status.
    Status { url: String, status: u16 },
    /// The response is not HTML, so there is no article to extract.
    NotHtml { url: String, content_type: String },
    /// The response body exceeded the byte cap and was abandoned mid-download.
    TooLarge { url: String, limit: u64 },
    /// The transport failed (connection, TLS, a malformed response).
    Transport { url: String, reason: String },
    /// The page was fetched, but its main content could not be extracted.
    Extraction { url: String, reason: String },
}

impl std::fmt::Display for WebError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WebError::InvalidUrl { url, reason } => {
                write!(f, "web: {url:?} is not a valid URL: {reason}")
            }
            WebError::UnsupportedScheme { url, scheme } => write!(
                f,
                "web: cannot fetch {url:?} — the {scheme:?} scheme is not supported; pass an http or \
                 https URL"
            ),
            WebError::BlockedAddress { url } => write!(
                f,
                "web: refused to fetch {url:?} — it resolves to a private or loopback address, which \
                 is not fetchable"
            ),
            WebError::Timeout { url } => {
                write!(
                    f,
                    "web: fetching {url:?} timed out; the page did not respond in time"
                )
            }
            WebError::Status { url, status } => {
                write!(f, "web: fetching {url:?} failed with HTTP status {status}")
            }
            WebError::NotHtml { url, content_type } => write!(
                f,
                "web: {url:?} is {content_type:?}, not an HTML page — web.markdown reads web pages, \
                 not other content types"
            ),
            WebError::TooLarge { url, limit } => write!(
                f,
                "web: {url:?} is larger than the {limit}-byte fetch limit and was not downloaded"
            ),
            WebError::Transport { url, reason } => {
                write!(f, "web: could not fetch {url:?}: {reason}")
            }
            WebError::Extraction { url, reason } => write!(
                f,
                "web: fetched {url:?} but could not extract its main content: {reason}"
            ),
        }
    }
}

impl std::error::Error for WebError {}

/// Whether a content type names HTML — `text/html` or `application/xhtml+xml`. The media type is
/// matched case-insensitively against its type/subtype, ignoring any parameters (a `; charset=…`
/// suffix), so the check is the one gate both the transport (early, from the response header) and the
/// pipeline (on a [`FetchedPage`], real or fake) apply.
pub(crate) fn is_html(content_type: &str) -> bool {
    let media = content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    media == "text/html" || media == "application/xhtml+xml"
}
