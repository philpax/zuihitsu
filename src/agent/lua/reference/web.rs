//! Web API reference entries: `web.markdown`.

use crate::agent::{
    api_doc::{ApiEntry, ApiEntry as AE, ApiGate, ApiType as AT},
    body_of,
};

/// The web entries, gated on the `browsing` feature.
pub(super) fn entries() -> Vec<ApiEntry> {
    let markdown = AE::new("web.markdown")
        .gated(ApiGate::Web)
        .description(body_of(include_str!("prose/web/markdown.md")))
        .required("url", AT::String, "the page URL (http or https)")
        .returns(AT::String);
    vec![markdown]
}
