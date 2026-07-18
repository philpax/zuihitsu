//! The one definition of a memory reference — the syntax that lets the agent, a connector, and the
//! console point at a specific memory (a person, an event, a place). A reference names a memory by its
//! immutable 26-character Crockford [`MemoryId`], carried in the canonical `[mem:<ulid>]` token, and
//! mirrors [`crate::turn_ref`] for a moment: [`construct`] mints the token, [`scan`] splits prose from
//! references, [`normalize`] collapses references to the canonical token, and [`extract_ids`] pulls the
//! ids out. The shared scanning skeleton lives in [`crate::ref_token`].
//!
//! # Deliberately token-only
//!
//! Unlike [`crate::turn_ref`], this module recognizes **no URL form**. A turn reference's URL carries
//! the turn's id directly (`?turn=<ulid>`), so the token/URL split described in `turn_ref` resolves
//! both forms to the same id with nothing but ULID parsing. A memory's console deep link, by contrast,
//! routes by *handle*, not by id — so recognizing it means matching a console route
//! and resolving a handle to a [`MemoryId`], which is the console frontend's own concern (route
//! knowledge plus a graph query), not this dependency-light core module's. So this module is the
//! canonical, agent-facing token vocabulary alone; the console maps its own URLs to these tokens.

use ulid::Ulid;

use crate::{ids::MemoryId, ref_token};

/// One span of scanned text: literal prose, or a resolved memory reference. A `Ref` covers the whole
/// `[mem:<ulid>]` token, so a renderer replaces it wholesale and a re-serialization ([`normalize`])
/// loses nothing but the reference's surface form.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Segment<'a> {
    /// Literal text that carries no reference.
    Prose(&'a str),
    /// A memory reference.
    Ref(MemoryId),
}

/// The canonical reference token for a memory: `[mem:<ulid>]`. What a connector splices in to point at
/// a memory, and the form every reference collapses to on [`normalize`].
pub fn construct(memory: MemoryId) -> String {
    format!("[{PREFIX_BODY}{}]", memory.0)
}

/// Split `text` into prose and memory references, in order. Recognizes only the canonical
/// `[mem:<ulid>]` token — no URL form (see the module documentation) — and treats a malformed id (a
/// `[mem:…]` whose body is not a ULID) as ordinary prose. Adjacent prose is coalesced, and empty prose
/// spans are dropped, so the segments read back as the original text.
pub fn scan(text: &str) -> Vec<Segment<'_>> {
    ref_token::scan(text, BRACKET_OPEN, parse_ulid, |_, _| None)
        .into_iter()
        .map(|segment| match segment {
            ref_token::Segment::Prose(prose) => Segment::Prose(prose),
            ref_token::Segment::Ref(memory) => Segment::Ref(memory),
        })
        .collect()
}

/// Rebuild `text` with every reference rendered as the canonical `[mem:<ulid>]` token. Prose is
/// preserved verbatim, so a message with no references is returned unchanged. A pasted bracket token is
/// already canonical, so this is a no-op on it — the console composer runs it to canonicalize any
/// bracket tokens a message already held, after mapping its own state-view URLs to tokens in the
/// frontend.
pub fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for segment in scan(text) {
        match segment {
            Segment::Prose(prose) => out.push_str(prose),
            Segment::Ref(memory) => out.push_str(&construct(memory)),
        }
    }
    out
}

/// Every memory id referenced in `text`, in order of appearance — the extract-all-ids path.
pub fn extract_ids(text: &str) -> Vec<MemoryId> {
    scan(text)
        .into_iter()
        .filter_map(|segment| match segment {
            Segment::Ref(memory) => Some(memory),
            Segment::Prose(_) => None,
        })
        .collect()
}

/// The literal body of the canonical token between the brackets: `[<PREFIX_BODY><ulid>]`.
const PREFIX_BODY: &str = "mem:";
/// The `[mem:` opener a bracket reference starts with.
const BRACKET_OPEN: &str = "[mem:";

/// Parse a Crockford ULID string into a [`MemoryId`], or `None` if it is not a valid ULID. The single
/// point where "does this denote a memory" is decided.
fn parse_ulid(body: &str) -> Option<MemoryId> {
    Ulid::from_string(body).ok().map(MemoryId)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// A `MemoryId` from a raw 128-bit value — proptest generates ids without touching the RNG-backed
    /// `MemoryId::generate`, so the round-trip covers the whole id space, including edge values.
    fn memory_id(bits: u128) -> MemoryId {
        MemoryId(Ulid::from(bits))
    }

    #[test]
    fn construct_is_the_canonical_bracket_token() {
        let memory = memory_id(1);
        let token = construct(memory);
        assert!(token.starts_with("[mem:"));
        assert!(token.ends_with(']'));
        assert_eq!(token.len(), "[mem:]".len() + 26);
    }

    #[test]
    fn scan_pulls_a_bracket_reference_out_of_prose() {
        let memory = memory_id(42);
        let text = format!("see {} for context", construct(memory));
        assert_eq!(
            scan(&text),
            vec![
                Segment::Prose("see "),
                Segment::Ref(memory),
                Segment::Prose(" for context"),
            ]
        );
    }

    #[test]
    fn construct_scan_normalize_round_trips() {
        let memory = memory_id(7);
        let text = format!("about {} please", construct(memory));
        assert_eq!(extract_ids(&text), vec![memory]);
        assert_eq!(normalize(&text), text);
    }

    #[test]
    fn ordinary_bracketed_prose_is_not_a_reference() {
        for text in [
            "[mem: whenever you like]",
            "[member of the club]",
            "[mem:short]",
            "a [note] in brackets",
            "mem:left then right",
        ] {
            assert!(extract_ids(text).is_empty(), "false positive on {text:?}");
            assert_eq!(scan(text), vec![Segment::Prose(text)]);
        }
    }

    #[test]
    fn a_malformed_ulid_body_is_rejected() {
        // 26 characters, but `I`, `L`, `O`, and `U` are not in the Crockford alphabet.
        let text = "[mem:IIIIIIIIIIIIIIIIIIIIIIIIII]";
        assert!(extract_ids(text).is_empty());
        assert_eq!(scan(text), vec![Segment::Prose(text)]);
    }

    #[test]
    fn no_url_form_is_recognized() {
        // Core recognizes no URL form: any URL stays prose. A frontend maps its own routes to
        // `[mem:<ulid>]` tokens before a message reaches this parser.
        let text = "look at http://host/some%2Fencoded%2Fpath?with=query for context";
        assert!(extract_ids(text).is_empty());
        assert_eq!(scan(text), vec![Segment::Prose(text)]);
    }

    #[test]
    fn multibyte_prose_scans_without_panicking() {
        // The scanner walks byte positions, so multibyte characters — an em dash, CJK, an emoji —
        // must never land it on a non-boundary slice (the wasm `unreachable` regression).
        let memory = memory_id(11);
        let text = format!(
            "さっき決めた — {} 🎉 “quotes” everywhere",
            construct(memory)
        );
        assert_eq!(extract_ids(&text), vec![memory]);
    }

    proptest! {
        /// Constructing then scanning a token round-trips to the same id, and nothing else.
        #[test]
        fn construct_scan_round_trips(bits in any::<u128>()) {
            let memory = memory_id(bits);
            let token = construct(memory);
            prop_assert_eq!(scan(&token), vec![Segment::Ref(memory)]);
            prop_assert_eq!(extract_ids(&token), vec![memory]);
        }

        /// References interleaved with reference-free prose extract in order.
        #[test]
        fn interleaved_references_extract_in_order(
            bits in proptest::collection::vec(any::<u128>(), 0..6),
        ) {
            let ids: Vec<MemoryId> = bits.iter().map(|b| memory_id(*b)).collect();
            let mut text = String::from("start ");
            for memory in &ids {
                text.push_str(&construct(*memory));
                text.push_str(" then ");
            }
            text.push_str("end");
            prop_assert_eq!(extract_ids(&text), ids);
        }

        /// Scanning arbitrary text — any Unicode, refs or none — never panics (the wasm `unreachable`
        /// regression), and ref-free text reassembles verbatim from its segments (the scanner is a pure
        /// split, lossy nowhere).
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
            let memory = memory_id(bits);
            let text = format!("{before} {} {after}", construct(memory));
            prop_assert!(extract_ids(&text).contains(&memory));
        }
    }
}
