//! The scanning primitives shared by the reference vocabularies — [`crate::turn_ref`] (a moment) and
//! [`crate::mem_ref`] (a memory). Each vocabulary names its subject by a 26-character Crockford ULID
//! carried in a canonical `[<prefix>:<ulid>]` bracket token; some also recognize a console deep-link
//! URL. The bracket matcher, the prose-flush/segment machinery, and the byte-walking scan loop are
//! identical across vocabularies, so they live here once, parameterized by the token prefix and an
//! id-parsing function. Each vocabulary keeps its own public `Segment`, its own constructor, and its
//! own URL semantics; this module owns only the shared skeleton.
//!
//! The URL-token boundary helpers ([`url_start_at`], [`url_token_end`]) are shared too, because a URL
//! runs to the same boundary whichever vocabulary owns the deep link.

/// One span of scanned text: literal prose, or a resolved reference carrying a typed id. A `Ref`
/// covers the whole matched token, so a re-serialization loses nothing but the reference's surface
/// form. Generic over the id type so each vocabulary's scan yields its own id newtype.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Segment<'a, Id> {
    /// Literal text that carries no reference.
    Prose(&'a str),
    /// A reference, however it was written.
    Ref(Id),
}

/// A Crockford ULID is exactly 26 characters (all ASCII), so a reference body is a fixed-width slice.
pub(crate) const ULID_LEN: usize = 26;

/// Split `text` into prose and references, in order. Recognizes the canonical `[<open><ulid>]` bracket
/// token (with `parse_body` deciding whether the body denotes a subject) and, at each byte, whatever
/// URL reference `url_at` matches — a vocabulary that recognizes no URLs passes a matcher that never
/// matches. A malformed bracket body stays prose. Adjacent prose is coalesced and empty prose spans
/// are dropped, so the segments read back as the original text.
pub(crate) fn scan<'a, Id>(
    text: &'a str,
    open: &str,
    parse_body: impl Fn(&str) -> Option<Id>,
    url_at: impl Fn(&str, usize) -> Option<(Id, usize)>,
) -> Vec<Segment<'a, Id>> {
    let mut segments = Vec::new();
    let bytes = text.as_bytes();
    let mut prose_start = 0;
    let mut i = 0;
    while i < bytes.len() {
        if let Some((id, len)) = bracket_ref_at(text, i, open, &parse_body) {
            flush_prose(&mut segments, &text[prose_start..i]);
            segments.push(Segment::Ref(id));
            i += len;
            prose_start = i;
            continue;
        }
        if let Some((id, len)) = url_at(text, i) {
            flush_prose(&mut segments, &text[prose_start..i]);
            segments.push(Segment::Ref(id));
            i += len;
            prose_start = i;
            continue;
        }
        i += 1;
    }
    flush_prose(&mut segments, &text[prose_start..]);
    segments
}

/// A bracket reference starting at byte `i`, if `text[i..]` is `[<open><ulid>]` — returning the parsed
/// id and the token's byte length. A body that `parse_body` rejects yields `None`, so `[turn: whenever]`
/// stays prose.
pub(crate) fn bracket_ref_at<Id>(
    text: &str,
    i: usize,
    open: &str,
    parse_body: impl Fn(&str) -> Option<Id>,
) -> Option<(Id, usize)> {
    let rest = text.get(i..)?.strip_prefix(open)?;
    // ULID characters are ASCII, so a 26-byte slice is exactly 26 characters.
    let body = rest.get(..ULID_LEN)?;
    if rest.as_bytes().get(ULID_LEN) != Some(&b']') {
        return None;
    }
    let id = parse_body(body)?;
    Some((id, open.len() + ULID_LEN + 1))
}

/// Push a prose segment, dropping it when empty so adjacent references leave no empty spans.
pub(crate) fn flush_prose<'a, Id>(segments: &mut Vec<Segment<'a, Id>>, prose: &'a str) {
    if !prose.is_empty() {
        segments.push(Segment::Prose(prose));
    }
}

/// Whether an `http://`/`https://` URL token begins at byte `i`, and at a token boundary (start of
/// text or after a non-alphanumeric byte) so a scheme embedded mid-word is not mistaken for a link.
/// Byte-wise on purpose: the scanner's `i` walks every byte, including the middle of a multibyte
/// character, where a `&text[i..]` slice would panic — matching on bytes cannot land off a boundary,
/// and a match guarantees `i` sits on ASCII `h`, a boundary, so the caller's slices are safe.
pub(crate) fn url_start_at(bytes: &[u8], i: usize) -> bool {
    if i > 0 && bytes[i - 1].is_ascii_alphanumeric() {
        return false;
    }
    bytes[i..].starts_with(b"http://") || bytes[i..].starts_with(b"https://")
}

/// The byte index one past a URL token starting at `i`: it runs to the next whitespace or URL
/// delimiter, then trailing sentence punctuation is returned to the prose (so `see …/foo/bar.`
/// keeps its period).
pub(crate) fn url_token_end(bytes: &[u8], i: usize) -> usize {
    let mut end = i;
    while end < bytes.len() && !is_url_terminator(bytes[end]) {
        end += 1;
    }
    while end > i && is_trailing_punctuation(bytes[end - 1]) {
        end -= 1;
    }
    end
}

/// A byte that cannot appear inside a URL, so it bounds a URL token: ASCII whitespace and the RFC 3986
/// delimiters a link never contains unescaped.
fn is_url_terminator(byte: u8) -> bool {
    byte.is_ascii_whitespace()
        || matches!(
            byte,
            b'<' | b'>' | b'"' | b'`' | b'{' | b'}' | b'|' | b'^' | b'\\'
        )
}

/// Trailing punctuation trimmed off a URL token so it reads as sentence punctuation, not part of the
/// link (a link glued to a `.`, a `,`, or a closing bracket).
fn is_trailing_punctuation(byte: u8) -> bool {
    matches!(
        byte,
        b'.' | b',' | b';' | b':' | b'!' | b'?' | b')' | b']' | b'}' | b'\'' | b'"'
    )
}
