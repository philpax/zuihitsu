//! The pure extraction pipeline: a fetched page → its main content as Markdown. Shared by the real
//! transport and the fake, so both run exactly the same extraction. No I/O, no clock — a pure
//! function of the [`FetchedPage`] and the character cap, so its behaviour is unit-testable without a
//! network.

use dom_smoothie::Readability;

use super::{FetchedPage, WebError, is_html};

/// The marker appended when the extracted Markdown is truncated, so the agent sees plainly that it is
/// reading a cut-down page rather than the whole thing.
const TRUNCATION_MARKER: &str =
    "\n\n[… content truncated: the page was longer than the fetch limit …]";

/// Extract `page`'s main content and render it as Markdown: readability strips the chrome (nav,
/// footers, cookie banners, sidebars), the body is converted to Markdown, the page title is prepended
/// as a top-level heading, and the result is truncated to `max_chars` with an explicit marker.
///
/// Rejects a non-HTML page with [`WebError::NotHtml`] — the same content-type gate the transport
/// applies early, re-checked here so a fake-injected page is held to the same contract.
pub(super) fn to_markdown(page: &FetchedPage, max_chars: usize) -> Result<String, WebError> {
    if !is_html(&page.content_type) {
        return Err(WebError::NotHtml {
            url: page.final_url.clone(),
            content_type: page.content_type.clone(),
        });
    }

    // Readability resolves relative links against the final (post-redirect) URL, so the extracted
    // content keeps working links rather than bare fragments.
    let mut readability = Readability::new(page.body.as_str(), Some(page.final_url.as_str()), None)
        .map_err(|error| WebError::Extraction {
            url: page.final_url.clone(),
            reason: error.to_string(),
        })?;
    let article = readability.parse().map_err(|error| WebError::Extraction {
        url: page.final_url.clone(),
        reason: error.to_string(),
    })?;

    let content_markdown =
        htmd::convert(article.content.as_ref()).map_err(|error| WebError::Extraction {
            url: page.final_url.clone(),
            reason: error.to_string(),
        })?;
    let content_markdown = strip_empty_links(&content_markdown);

    let title = article.title.trim();
    let body = content_markdown.trim();
    let mut markdown = String::new();
    if !title.is_empty() {
        markdown.push_str("# ");
        markdown.push_str(title);
        markdown.push_str("\n\n");
    }
    markdown.push_str(body);

    Ok(truncate(markdown, max_chars))
}

/// Drop empty links — `[](…)` with no text — from converted Markdown, collapsing any blank lines the
/// removal leaves behind. Heading-permalink anchors (GitHub's `[](#section)` under every heading) and
/// other icon-only links convert to exactly this shape, and with no text they render as nothing while
/// still spending the reader's attention. A link with text is never touched.
fn strip_empty_links(markdown: &str) -> String {
    let mut out = String::with_capacity(markdown.len());
    let mut rest = markdown;
    while let Some(start) = rest.find("[](") {
        out.push_str(&rest[..start]);
        let after_open = &rest[start + 3..];
        // Keep the occurrence verbatim when no closing parenthesis follows — it is not a link.
        match after_open.find(')') {
            Some(close) => rest = &after_open[close + 1..],
            None => {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    // Normalise blank runs document-wide to a single blank line: it heals the gaps left where an
    // empty link was a paragraph of its own, and extra vertical space carries nothing for a reader
    // of extracted prose anyway.
    let mut collapsed = String::with_capacity(out.len());
    let mut blank_run = 0;
    for line in out.lines() {
        if line.trim().is_empty() {
            blank_run += 1;
            if blank_run > 1 {
                continue;
            }
        } else {
            blank_run = 0;
        }
        collapsed.push_str(line);
        collapsed.push('\n');
    }
    collapsed.truncate(collapsed.trim_end().len());
    collapsed
}

/// Truncate `markdown` to at most `max_chars` characters, appending the truncation marker when it was
/// cut. Counts characters, not bytes, so a multi-byte page is never sliced mid-character. A `0` cap is
/// treated as "no cap" — the whole document passes through — so a misconfigured zero does not silently
/// erase every fetch.
fn truncate(markdown: String, max_chars: usize) -> String {
    if max_chars == 0 || markdown.chars().count() <= max_chars {
        return markdown;
    }
    let mut truncated: String = markdown.chars().take(max_chars).collect();
    truncated.push_str(TRUNCATION_MARKER);
    truncated
}

#[cfg(test)]
mod tests {
    use super::{TRUNCATION_MARKER, to_markdown, truncate};
    use crate::web::FetchedPage;

    /// A chrome-heavy page: a site header with navigation, a cookie banner, a sidebar of unrelated
    /// links, the real article body, and a footer. Extraction should keep the article prose and drop
    /// the surrounding furniture.
    const CHROME_HEAVY_HTML: &str = include_str!("fixtures/chrome_heavy.html");

    fn html_page(body: &str) -> FetchedPage {
        FetchedPage {
            final_url: "https://example.com/lumen-ledger".to_owned(),
            content_type: "text/html; charset=utf-8".to_owned(),
            body: body.to_owned(),
        }
    }

    #[test]
    fn extraction_keeps_the_article_and_drops_the_chrome() {
        let markdown = to_markdown(&html_page(CHROME_HEAVY_HTML), 20_000).unwrap();
        // The article prose survives.
        assert!(
            markdown.contains("double-entry bookkeeping"),
            "article prose missing: {markdown}"
        );
        assert!(
            markdown.contains("deterministic ledger engine"),
            "article prose missing: {markdown}"
        );
        // The chrome is gone.
        for chrome in [
            "This site uses cookies",
            "Sign in",
            "Sponsored: buy our newsletter",
            "All rights reserved",
            "Privacy policy",
        ] {
            assert!(
                !markdown.contains(chrome),
                "chrome leaked into the extraction: {chrome:?}\n{markdown}"
            );
        }
    }

    #[test]
    fn the_title_is_prepended_as_a_heading() {
        let markdown = to_markdown(&html_page(CHROME_HEAVY_HTML), 20_000).unwrap();
        assert!(
            markdown.starts_with("# The Lumen Ledger — Project Overview"),
            "expected a title heading, got: {markdown}"
        );
    }

    #[test]
    fn a_non_html_page_is_rejected() {
        let page = FetchedPage {
            final_url: "https://example.com/data.json".to_owned(),
            content_type: "application/json".to_owned(),
            body: "{\"not\":\"html\"}".to_owned(),
        };
        assert!(matches!(
            to_markdown(&page, 20_000),
            Err(crate::web::WebError::NotHtml { .. })
        ));
    }

    #[test]
    fn a_long_extraction_is_truncated_with_a_marker() {
        let markdown = to_markdown(&html_page(CHROME_HEAVY_HTML), 200).unwrap();
        assert!(
            markdown.ends_with(TRUNCATION_MARKER),
            "expected a truncation marker, got: {markdown}"
        );
        // The cap counts the content characters, not the appended marker.
        let content_len = markdown.chars().count() - TRUNCATION_MARKER.chars().count();
        assert_eq!(content_len, 200);
    }

    #[test]
    fn empty_links_are_stripped_and_text_links_kept() {
        // The shape GitHub renders under every heading: an empty permalink anchor on its own line.
        let converted = "# Title\n\n[](#title)\n\nProse with an inline [](#anchor) artifact, and a \
                         [real link](https://example.com) that stays.\n\n[](no-close";
        let cleaned = super::strip_empty_links(converted);
        assert!(!cleaned.contains("[](#"), "empty links removed: {cleaned}");
        assert!(
            cleaned.contains("[real link](https://example.com)"),
            "a link with text is untouched: {cleaned}"
        );
        assert!(
            !cleaned.contains("\n\n\n"),
            "blank runs are collapsed: {cleaned:?}"
        );
        assert!(
            cleaned.contains("[](no-close"),
            "an unclosed occurrence is not a link and stays verbatim: {cleaned}"
        );
    }

    #[test]
    fn truncate_leaves_short_text_untouched_and_treats_zero_as_no_cap() {
        assert_eq!(truncate("short".to_owned(), 100), "short");
        // A zero cap passes the whole document through rather than erasing it.
        assert_eq!(truncate("keep everything".to_owned(), 0), "keep everything");
    }
}
