//! The two shared helpers for embedding agent-facing prose from `.md` files: stripping the trailing
//! newline `include_str!` carries, and substituting `{{placeholder}}` names at the use site.

/// Strip the trailing newline `include_str!` carries, so an embedded body matches the original
/// literal, which ended at its last character.
pub fn body_of(raw: &str) -> String {
    raw.trim_end_matches('\n').to_owned()
}

/// Substitute `{{key}}` placeholders in an embedded body with the given values, in order. A plain
/// ordered replace pass, mirroring the templates' `.replace` chain but data-driven. Accepts both an
/// `&str` and an owned `String` body (`body_of(include_str!(...))`), whichever the call site finds
/// clearer.
pub fn render_placeholders(raw: impl AsRef<str>, subs: &[(&str, &str)]) -> String {
    let mut out = raw.as_ref().to_owned();
    for (key, value) in subs {
        out = out.replace(&format!("{{{{{key}}}}}"), value);
    }
    out
}
