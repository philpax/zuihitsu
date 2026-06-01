//! The fetcher seam: retrieve a page as Markdown. The real fetcher (lightpanda + markitdown, with
//! the resolved-IP egress guard) lands in Stage 11; tests supply canned pages and (later) exercise
//! the egress guard without real DNS or sockets (spec §Testability, §Lua API → async).

use std::collections::HashMap;

use async_trait::async_trait;

/// Retrieves a URL and returns its content as Markdown.
#[async_trait]
pub trait Fetcher: Send + Sync {
    async fn fetch_page(&self, url: &str) -> Result<String, FetchError>;
}

/// A fetch failure.
#[derive(Debug)]
pub enum FetchError {
    /// No content is available for the URL (the canned fetcher has nothing for it).
    NotFound,
    /// The URL was rejected — a bad scheme, or the egress guard (Stage 11).
    Blocked(String),
    /// The backend (browser, network) failed.
    Backend(String),
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FetchError::NotFound => write!(f, "fetch: no content is available for that URL"),
            FetchError::Blocked(reason) => write!(f, "fetch: the URL was rejected: {reason}"),
            FetchError::Backend(message) => write!(f, "fetch: {message}"),
        }
    }
}

impl std::error::Error for FetchError {}

/// A fake serving canned pages by exact URL. Exercises callers, and later the egress guard, with
/// no real DNS or sockets.
#[derive(Default)]
pub struct CannedFetcher {
    pages: HashMap<String, String>,
}

impl CannedFetcher {
    pub fn new() -> CannedFetcher {
        CannedFetcher::default()
    }

    /// Register a page; chainable for terse test setup.
    pub fn with_page(
        mut self,
        url: impl Into<String>,
        markdown: impl Into<String>,
    ) -> CannedFetcher {
        self.pages.insert(url.into(), markdown.into());
        self
    }
}

#[async_trait]
impl Fetcher for CannedFetcher {
    async fn fetch_page(&self, url: &str) -> Result<String, FetchError> {
        self.pages.get(url).cloned().ok_or(FetchError::NotFound)
    }
}
