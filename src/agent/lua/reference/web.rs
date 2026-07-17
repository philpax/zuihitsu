//! Web API reference entries: `web.markdown`.

use crate::agent::api_doc::{ApiEntry, ApiEntry as AE, ApiGate, ApiType as AT};

/// The web entries, gated on the `browsing` feature.
pub(super) fn entries() -> Vec<ApiEntry> {
    let markdown = AE::new("web.markdown")
        .gated(ApiGate::Web)
        .description(
            "Fetch a web page and return its main content as Markdown. The page's chrome — \
             navigation, sidebars, cookie banners, footers — is stripped, leaving the article text \
             under a title heading. Reads only http/https HTML pages; a non-HTML URL, a private or \
             loopback address, a bad status, or a timeout comes back as an error you can act on. Long \
             pages are truncated with a marker. Fetch to read, then summarize what matters into memory \
             — do not paste the whole page in. Keep the source URL alongside your summary, so a later \
             question about a detail you left out can re-read the page.",
        )
        .required("url", AT::String, "the page URL (http or https)")
        .returns(AT::String);
    vec![markdown]
}
