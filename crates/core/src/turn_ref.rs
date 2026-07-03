//! The one definition of a turn reference — the syntax that lets the agent and the console point back
//! to a specific conversation moment (spec §Conversations → Transcripts).
//!
//! A reference names a `ConversationTurn` by its 26-character Crockford [`TurnId`], carried in one of
//! two forms: the canonical `[turn:<ulid>]` token, or the `?turn=<ulid>` query parameter of a console
//! deep-link URL. Both forms parse to the same [`TurnId`] through this module — one parser is the
//! whole definition of "what counts as a turn reference", so a renderer, an extract-all-ids caller,
//! and the composer's URL-normalizer never drift on the syntax. [`construct`] mints the canonical
//! token, [`scan`] splits prose from references (recognizing both forms), [`normalize`] collapses
//! every reference to the canonical token, and [`extract_ids`] pulls the ids out.
//!
//! The module is deliberately dependency-light — no URL crate, no regex, only ULID parsing — so it
//! compiles to wasm and drives the console through `console-wasm`.

use ulid::Ulid;

use crate::ids::TurnId;

/// One span of scanned text: literal prose, or a resolved turn reference. A `Ref` covers the whole
/// matched token (a `[turn:…]` bracket or a `?turn=…` URL), so a renderer replaces the token wholesale
/// and a re-serialization ([`normalize`]) loses nothing but the reference's surface form.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Segment<'a> {
    /// Literal text that carries no reference.
    Prose(&'a str),
    /// A turn reference, however it was written.
    Ref(TurnId),
}

/// The canonical reference token for `turn`: `[turn:<ulid>]`. What the agent copies to cite a moment,
/// and the form every reference collapses to on [`normalize`].
pub fn construct(turn: TurnId) -> String {
    format!("[{PREFIX_BODY}{}]", turn.0)
}

/// Split `text` into prose and turn references, in order. Recognizes both the canonical `[turn:<ulid>]`
/// token and a console deep-link URL carrying `?turn=<ulid>` (so a pasted link and a copied token both
/// resolve), and treats a malformed id — a `[turn:…]` whose body is not a ULID, or a URL whose `turn`
/// value is not a ULID — as ordinary prose, never a reference. Adjacent prose is coalesced, and empty
/// prose spans are dropped, so the segments read back as the original text.
pub fn scan(text: &str) -> Vec<Segment<'_>> {
    let mut segments = Vec::new();
    let bytes = text.as_bytes();
    let mut prose_start = 0;
    let mut i = 0;
    while i < bytes.len() {
        if let Some((turn, len)) = bracket_ref_at(text, i) {
            flush_prose(&mut segments, &text[prose_start..i]);
            segments.push(Segment::Ref(turn));
            i += len;
            prose_start = i;
            continue;
        }
        if url_start_at(bytes, i) {
            let end = url_token_end(bytes, i);
            if let Some(turn) = url_ref(&text[i..end]) {
                flush_prose(&mut segments, &text[prose_start..i]);
                segments.push(Segment::Ref(turn));
                i = end;
                prose_start = i;
                continue;
            }
        }
        i += 1;
    }
    flush_prose(&mut segments, &text[prose_start..]);
    segments
}

/// Rebuild `text` with every reference rendered as the canonical `[turn:<ulid>]` token — collapsing a
/// pasted console URL to the token every downstream consumer expects (the composer's send-time
/// normalization). Prose is preserved verbatim, so a message with no references is returned unchanged.
pub fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for segment in scan(text) {
        match segment {
            Segment::Prose(prose) => out.push_str(prose),
            Segment::Ref(turn) => out.push_str(&construct(turn)),
        }
    }
    out
}

/// Every turn id referenced in `text`, in order of appearance — the extract-all-ids path (the console's
/// pretty projection, an agent's "resolve every link pasted here").
pub fn extract_ids(text: &str) -> Vec<TurnId> {
    scan(text)
        .into_iter()
        .filter_map(|segment| match segment {
            Segment::Ref(turn) => Some(turn),
            Segment::Prose(_) => None,
        })
        .collect()
}

/// The turn id carried in `url`'s query string as `turn=<ulid>`, if any — the single definition of a
/// deep-link's turn reference, shared by [`scan`] (which finds URL tokens in prose) and any caller
/// holding an isolated URL. The fragment (`#…`) is stripped first, then each `&`-separated pair is
/// checked for a `turn=` key whose value parses as a ULID; other parameters are ignored, so a link
/// with `?foo=1&turn=<ulid>&bar=2` resolves.
pub fn url_ref(url: &str) -> Option<TurnId> {
    let without_fragment = url.split_once('#').map_or(url, |(head, _)| head);
    let (_, query) = without_fragment.split_once('?')?;
    query
        .split('&')
        .find_map(|pair| pair.strip_prefix("turn="))
        .and_then(parse_ulid)
}

/// The literal body of the canonical token between the brackets: `[<PREFIX_BODY><ulid>]`.
const PREFIX_BODY: &str = "turn:";
/// The `[turn:` opener a bracket reference starts with.
const BRACKET_OPEN: &str = "[turn:";
/// A Crockford ULID is exactly 26 characters (all ASCII), so a reference body is a fixed-width slice.
const ULID_LEN: usize = 26;

/// A bracket reference starting at byte `i`, if `text[i..]` is `[turn:<ulid>]` — returning the parsed
/// id and the token's byte length. A non-ULID body yields `None`, so `[turn: whenever]` stays prose.
fn bracket_ref_at(text: &str, i: usize) -> Option<(TurnId, usize)> {
    let rest = text.get(i..)?.strip_prefix(BRACKET_OPEN)?;
    // ULID characters are ASCII, so a 26-byte slice is exactly 26 characters.
    let body = rest.get(..ULID_LEN)?;
    if rest.as_bytes().get(ULID_LEN) != Some(&b']') {
        return None;
    }
    let turn = parse_ulid(body)?;
    Some((turn, BRACKET_OPEN.len() + ULID_LEN + 1))
}

/// Whether an `http://`/`https://` URL token begins at byte `i`, and at a token boundary (start of
/// text or after a non-alphanumeric byte) so a scheme embedded mid-word is not mistaken for a link.
/// Byte-wise on purpose: the scanner's `i` walks every byte, including the middle of a multibyte
/// character, where a `&text[i..]` slice would panic — matching on bytes cannot land off a boundary,
/// and a match guarantees `i` sits on ASCII `h`, a boundary, so the caller's slices are safe.
fn url_start_at(bytes: &[u8], i: usize) -> bool {
    if i > 0 && bytes[i - 1].is_ascii_alphanumeric() {
        return false;
    }
    bytes[i..].starts_with(b"http://") || bytes[i..].starts_with(b"https://")
}

/// The byte index one past a URL token starting at `i`: it runs to the next whitespace or URL
/// delimiter, then trailing sentence punctuation is returned to the prose (so `see …?turn=<ulid>.`
/// keeps its period).
fn url_token_end(bytes: &[u8], i: usize) -> usize {
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

/// Parse a Crockford ULID string into a [`TurnId`], or `None` if it is not a valid ULID. The single
/// point where "does this denote a turn" is decided, so both reference forms reject the same way.
fn parse_ulid(body: &str) -> Option<TurnId> {
    Ulid::from_string(body).ok().map(TurnId)
}

/// Push a prose segment, dropping it when empty so adjacent references leave no empty spans.
fn flush_prose<'a>(segments: &mut Vec<Segment<'a>>, prose: &'a str) {
    if !prose.is_empty() {
        segments.push(Segment::Prose(prose));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// A `TurnId` from a raw 128-bit value — proptest generates ids without touching the RNG-backed
    /// `TurnId::generate`, so the round-trip covers the whole id space, including edge values.
    fn turn_id(bits: u128) -> TurnId {
        TurnId(Ulid::from(bits))
    }

    #[test]
    fn construct_is_the_canonical_bracket_token() {
        let turn = turn_id(1);
        let token = construct(turn);
        assert!(token.starts_with("[turn:"));
        assert!(token.ends_with(']'));
        assert_eq!(token.len(), "[turn:]".len() + ULID_LEN);
    }

    #[test]
    fn scan_pulls_a_bracket_reference_out_of_prose() {
        let turn = turn_id(42);
        let text = format!("as you said {} earlier", construct(turn));
        assert_eq!(
            scan(&text),
            vec![
                Segment::Prose("as you said "),
                Segment::Ref(turn),
                Segment::Prose(" earlier"),
            ]
        );
    }

    #[test]
    fn scan_resolves_a_console_url_and_normalize_collapses_it() {
        let turn = turn_id(7);
        let url = format!("http://127.0.0.1:7878/discord/planning?turn={}", turn.0);
        let text = format!("remind me about {url} please");
        assert_eq!(extract_ids(&text), vec![turn]);
        assert_eq!(
            normalize(&text),
            format!("remind me about {} please", construct(turn))
        );
    }

    #[test]
    fn url_ref_keeps_other_query_parameters_intact() {
        let turn = turn_id(99);
        let url = format!("https://host/room?foo=1&turn={}&bar=2#frag", turn.0);
        assert_eq!(url_ref(&url), Some(turn));
    }

    #[test]
    fn trailing_punctuation_stays_prose_not_part_of_the_link() {
        let turn = turn_id(3);
        let url = format!("https://host/room?turn={}", turn.0);
        let text = format!("see {url}.");
        assert_eq!(
            scan(&text),
            vec![
                Segment::Prose("see "),
                Segment::Ref(turn),
                Segment::Prose("."),
            ]
        );
    }

    #[test]
    fn ordinary_bracketed_prose_is_not_a_reference() {
        for text in [
            "[turn: whenever you like]",
            "[turned around]",
            "[turn:short]",
            "a [note] in brackets",
            "turn:left then right",
        ] {
            assert!(extract_ids(text).is_empty(), "false positive on {text:?}");
            assert_eq!(scan(text), vec![Segment::Prose(text)]);
        }
    }

    #[test]
    fn a_malformed_ulid_body_is_rejected() {
        // 26 characters, but `I`, `L`, `O`, and `U` are not in the Crockford alphabet.
        let text = "[turn:IIIIIIIIIIIIIIIIIIIIIIIIII]";
        assert!(extract_ids(text).is_empty());
        assert_eq!(scan(text), vec![Segment::Prose(text)]);
    }

    #[test]
    fn a_url_without_a_turn_parameter_stays_prose() {
        let text = "look at https://host/room?foo=bar for context";
        assert_eq!(scan(text), vec![Segment::Prose(text)]);
    }

    #[test]
    fn multibyte_prose_scans_without_panicking() {
        // The scanner walks byte positions, so multibyte characters — an em dash, CJK, an emoji —
        // must never land it on a non-boundary slice (the wasm `unreachable` regression).
        let turn = turn_id(11);
        let text = format!("さっき決めた — {} 🎉 “quotes” everywhere", construct(turn));
        assert_eq!(extract_ids(&text), vec![turn]);
        let url = format!("https://host/room?turn={}", turn.0);
        let text = format!("見て {url} — please");
        assert_eq!(extract_ids(&text), vec![turn]);
    }

    proptest! {
        /// Constructing then scanning a token round-trips to the same id, and nothing else.
        #[test]
        fn construct_scan_round_trips(bits in any::<u128>()) {
            let turn = turn_id(bits);
            let token = construct(turn);
            prop_assert_eq!(scan(&token), vec![Segment::Ref(turn)]);
            prop_assert_eq!(extract_ids(&token), vec![turn]);
        }

        /// References interleaved with reference-free prose extract in order, whichever form is used.
        #[test]
        fn interleaved_references_extract_in_order(
            bits in proptest::collection::vec(any::<u128>(), 0..6),
            urls in proptest::collection::vec(any::<bool>(), 0..6),
        ) {
            let ids: Vec<TurnId> = bits.iter().map(|b| turn_id(*b)).collect();
            let mut text = String::from("start ");
            for (index, turn) in ids.iter().enumerate() {
                if urls.get(index).copied().unwrap_or(false) {
                    text.push_str(&format!("https://host/r?turn={} ", turn.0));
                } else {
                    text.push_str(&construct(*turn));
                    text.push_str(" then ");
                }
            }
            text.push_str("end");
            prop_assert_eq!(extract_ids(&text), ids);
        }

        /// A URL's turn parameter round-trips regardless of surrounding parameters.
        #[test]
        fn url_ref_round_trips(bits in any::<u128>()) {
            let turn = turn_id(bits);
            let url = format!("https://host/room?a=1&turn={}&b=2", turn.0);
            prop_assert_eq!(url_ref(&url), Some(turn));
        }

        /// Scanning arbitrary text — any Unicode, refs or none — never panics (the wasm
        /// `unreachable` regression), and ref-free text reassembles verbatim from its segments
        /// (the scanner is a pure split, lossy nowhere).
        #[test]
        fn scan_is_total_and_lossless(text in any::<String>()) {
            let segments = scan(&text);
            if extract_ids(&text).is_empty() {
                let rebuilt: String = segments
                    .iter()
                    .map(|segment| match segment {
                        Segment::Prose(prose) => *prose,
                        Segment::Ref(_) => unreachable!("no ids extracted"),
                    })
                    .collect();
                prop_assert_eq!(rebuilt, text);
            }
        }

        /// A reference embedded in arbitrary Unicode prose still extracts (boundary-safe scanning).
        #[test]
        fn a_ref_survives_arbitrary_surrounding_prose(
            bits in any::<u128>(),
            before in any::<String>(),
            after in any::<String>(),
        ) {
            let turn = turn_id(bits);
            let text = format!("{before} {} {after}", construct(turn));
            prop_assert!(extract_ids(&text).contains(&turn));
        }
    }
}
