//! A scriptable in-memory [`WebFetcher`] for tests and the eval harness — the transport-seam fake.
//! A test registers a canned [`FetchedPage`] (or a scripted failure) per URL, so a `web.markdown`
//! test needs no network. The pure pipeline still runs over the canned page, so a fake exercises the
//! same extraction the real fetcher drives. Mirrors `FakeMcpHost`.

use std::collections::HashMap;

use async_trait::async_trait;

use super::{FetchedPage, WebError, WebFetcher};

/// A scriptable [`WebFetcher`]: a map from URL to the canned page (or error) it returns.
#[derive(Clone, Default)]
pub struct FakeWebFetcher {
    pages: HashMap<String, Result<FetchedPage, WebError>>,
}

impl FakeWebFetcher {
    pub fn new() -> FakeWebFetcher {
        FakeWebFetcher::default()
    }

    /// Register `url` to return an HTML page with `html` as its body (content type `text/html`), its
    /// final URL echoing the request. Chainable.
    pub fn with_html(mut self, url: impl Into<String>, html: impl Into<String>) -> FakeWebFetcher {
        let url = url.into();
        self.pages.insert(
            url.clone(),
            Ok(FetchedPage {
                final_url: url,
                content_type: "text/html; charset=utf-8".to_owned(),
                body: html.into(),
            }),
        );
        self
    }

    /// Register `url` to return `page` verbatim — for a test that needs a specific content type or a
    /// post-redirect final URL. Chainable.
    pub fn with_page(mut self, url: impl Into<String>, page: FetchedPage) -> FakeWebFetcher {
        self.pages.insert(url.into(), Ok(page));
        self
    }

    /// Register `url` to fail with `error`. Chainable.
    pub fn with_error(mut self, url: impl Into<String>, error: WebError) -> FakeWebFetcher {
        self.pages.insert(url.into(), Err(error));
        self
    }
}

#[async_trait]
impl WebFetcher for FakeWebFetcher {
    async fn fetch(&self, url: &str) -> Result<FetchedPage, WebError> {
        match self.pages.get(url) {
            Some(result) => result.clone(),
            // An unregistered URL is the fake's equivalent of an unreachable host — a transport error
            // the caller can distinguish from a scripted failure.
            None => Err(WebError::Transport {
                url: url.to_owned(),
                reason: "no page is registered for this URL in the fake fetcher".to_owned(),
            }),
        }
    }
}
