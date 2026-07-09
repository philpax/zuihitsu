//! The free-text placeholder guard: a scanner that spots a literal `{ident}`-shaped placeholder in a
//! string a script hands the API, and the check helper the call sites apply at the Lua argument
//! boundary. A small model sometimes writes string-format syntax — `mem:append("Full text: {content}")`
//! — inside a plain quoted string, which records the literal `{content}` instead of the value; the
//! taught interpolation path is a backtick string, which does render the variable. The guard raises a
//! teachable [`PlaceholderError`] at the point of failure. Applied only where the script's own text
//! crosses into the API, so genesis and console writes — which never pass through a script — are
//! naturally exempt.

use super::super::error::PlaceholderError;

/// Reject `text` when it carries a literal `{ident}` placeholder — string-format syntax that a
/// plain quoted string never interpolates, so the uninterpolated braces would be stored (or
/// searched) as fact. `what` names the argument for the error's wording ("entry text",
/// "memory name", …). Applied at the Lua argument boundary only, so genesis and console writes
/// — which never pass through a script — may carry literal braces.
pub(crate) fn check_interpolated(what: &'static str, text: &str) -> mlua::Result<()> {
    match uninterpolated_placeholder(text) {
        Some(placeholder) => Err(PlaceholderError {
            what,
            placeholder: placeholder.to_owned(),
        }
        .into()),
        None => Ok(()),
    }
}

/// The first `{ident}`-shaped placeholder in `text`, if any: `{` immediately followed by a Luau
/// identifier and an optional accessor chain (`.field`, `:method`, `[index]`, a trailing `()`),
/// then `}` — no whitespace anywhere inside. Matches the string-format slips (`{content}`,
/// `{e.text}`, `{es[1]}`) while passing braces that are not expression-shaped (`{}`,
/// `{ day = "…" }`, prose).
fn uninterpolated_placeholder(text: &str) -> Option<&str> {
    let bytes = text.as_bytes();
    let mut start = 0;
    while start < bytes.len() {
        if bytes[start] == b'{'
            && let Some(end) = match_placeholder(bytes, start)
        {
            return Some(&text[start..end]);
        }
        start += 1;
    }
    None
}

/// Match a placeholder beginning at the `{` at `open`, returning the index just past its closing `}`
/// on success. The interior is a leading identifier followed by zero or more accessors, with no
/// whitespace anywhere and the closing `}` immediately after. Returns `None` when the interior does
/// not fit the grammar, so the caller resumes at the next byte rather than skipping the interior.
fn match_placeholder(bytes: &[u8], open: usize) -> Option<usize> {
    let mut pos = match_identifier(bytes, open + 1)?;
    loop {
        match bytes.get(pos) {
            Some(b'}') => return Some(pos + 1),
            Some(b'.') | Some(b':') => pos = match_identifier(bytes, pos + 1)?,
            Some(b'[') => pos = match_index(bytes, pos + 1)?,
            Some(b'(') if bytes.get(pos + 1) == Some(&b')') => pos += 2,
            _ => return None,
        }
    }
}

/// Match a Luau identifier (`[A-Za-z_][A-Za-z0-9_]*`) beginning at `pos`, returning the index just
/// past it, or `None` when `pos` does not start one.
fn match_identifier(bytes: &[u8], pos: usize) -> Option<usize> {
    match bytes.get(pos) {
        Some(b) if b.is_ascii_alphabetic() || *b == b'_' => {}
        _ => return None,
    }
    let mut end = pos + 1;
    while let Some(b) = bytes.get(end) {
        if b.is_ascii_alphanumeric() || *b == b'_' {
            end += 1;
        } else {
            break;
        }
    }
    Some(end)
}

/// Match an index body — one or more characters that are not whitespace, `]`, `{`, or `}` — followed
/// by its closing `]`, beginning just past the opening `[` at `pos`. Returns the index just past the
/// `]`, or `None` when the body is empty or unterminated.
fn match_index(bytes: &[u8], pos: usize) -> Option<usize> {
    let mut end = pos;
    while let Some(b) = bytes.get(end) {
        if b.is_ascii_whitespace() || matches!(b, b']' | b'{' | b'}') {
            break;
        }
        end += 1;
    }
    if end == pos || bytes.get(end) != Some(&b']') {
        return None;
    }
    Some(end + 1)
}

#[cfg(test)]
mod tests {
    use super::{check_interpolated, uninterpolated_placeholder};

    #[test]
    fn matches_expression_shaped_placeholders() {
        for text in [
            "{content}",
            "{e.text}",
            "{es[1]}",
            "{e:render()}",
            "prefix {content} suffix",
        ] {
            assert!(
                uninterpolated_placeholder(text).is_some(),
                "expected a match in {text:?}"
            );
            assert!(
                check_interpolated("entry text", text).is_err(),
                "expected a rejection for {text:?}"
            );
        }
    }

    #[test]
    fn passes_braces_that_are_not_expression_shaped() {
        for text in [
            "{}",
            "{ day = \"2026-06-03\" }",
            "{not a placeholder}",
            "{1, 2, 3}",
            "{123}",
            "plain text with no braces",
            "a { b } c",
        ] {
            assert!(
                uninterpolated_placeholder(text).is_none(),
                "expected no match in {text:?}"
            );
            assert!(
                check_interpolated("entry text", text).is_ok(),
                "expected acceptance for {text:?}"
            );
        }
    }

    #[test]
    fn returns_the_first_of_several() {
        assert_eq!(
            uninterpolated_placeholder("{first} then {second}"),
            Some("{first}")
        );
    }

    #[test]
    fn resumes_after_a_non_matching_open_brace() {
        // The leading `{ ...` does not match (whitespace inside), so the scan resumes and finds the
        // later expression-shaped placeholder rather than skipping past it.
        assert_eq!(
            uninterpolated_placeholder("{ not one } but {content}"),
            Some("{content}")
        );
    }
}
