//! URL extraction for the ambient recall pass: a minimal, scheme-anchored scan that pulls the http(s)
//! links an inbound message carries, so the hint can point at reading them rather than leaving a shared
//! link inert. Dedup and the cap happen in the caller.

/// Extract the http(s) URLs an inbound message carries, in order of appearance. The scan is minimal and
/// scheme-anchored: from each `http://` or `https://` it takes characters up to the first ASCII
/// whitespace or a closing delimiter that bounds a URL embedded in prose, markdown, or brackets, then
/// strips trailing sentence punctuation. A bare scheme with no host is discarded. A missed exotic form
/// costs nothing — the pointer is a nudge, not a parser. Dedup and the cap happen in the caller.
pub(super) fn extract_urls(text: &str) -> Vec<String> {
    let mut urls = Vec::new();
    let mut search_from = 0;
    while let Some(start) = find_scheme(text, search_from) {
        let rest = &text[start..];
        let scheme_len = if rest.starts_with("https://") { 8 } else { 7 };
        // Take the span from the scheme up to the first bounding character.
        let span_end = rest[scheme_len..]
            .find(|c: char| c.is_ascii_whitespace() || is_url_boundary(c))
            .map(|offset| scheme_len + offset)
            .unwrap_or(rest.len());
        let span = &rest[..span_end];
        let trimmed = span.trim_end_matches(['.', ',', ';', ':', '!', '?', ')', ']', '>']);
        // Keep only a URL that carries a host after the scheme.
        if trimmed.len() > scheme_len {
            urls.push(trimmed.to_owned());
        }
        search_from = start + span_end;
    }
    urls
}

/// The byte index at or after `from` where the next `http://` or `https://` scheme begins, or `None`.
/// `str::find` returns a char-boundary index, so the caller may slice `text` at it safely.
fn find_scheme(text: &str, from: usize) -> Option<usize> {
    let haystack = &text[from..];
    match (haystack.find("http://"), haystack.find("https://")) {
        (Some(a), Some(b)) => Some(from + a.min(b)),
        (Some(a), None) => Some(from + a),
        (None, Some(b)) => Some(from + b),
        (None, None) => None,
    }
}

/// The characters that bound a URL embedded in prose, markdown, or brackets — the closing side of a
/// wrapping pair, or a shell/markdown metacharacter that never appears mid-URL in practice.
fn is_url_boundary(c: char) -> bool {
    matches!(
        c,
        '<' | '>' | '"' | '\'' | '`' | ')' | ']' | '}' | '|' | '\\'
    )
}
