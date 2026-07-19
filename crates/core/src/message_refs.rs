//! The one parser for the reference vocabularies a message carries — a turn reference (a moment) and a
//! memory reference (a person, an event, a place). Each names its subject by a 26-character Crockford
//! ULID inside a canonical `[<prefix>:<ulid>]` bracket token: `[turn:<ulid>]` for a [`TurnId`] and
//! `[mem:<ulid>]` for a [`MemoryId`]. One pass over the message decomposes it into typed [`Segment`]s
//! covering both grammars, so a renderer, an extract-all-ids caller, and the composer's normalizer all
//! read the same definition of "what counts as a reference" and never drift.
//!
//! [`scan`] splits prose from references, [`normalize`] collapses every reference to its canonical
//! token, and [`extract_turn_ids`]/[`extract_mem_ids`] pull the ids of one kind out. The canonical
//! tokens are minted by [`crate::turn_ref::construct`] and [`crate::mem_ref::construct`], the vocabulary
//! constructors this parser reconstructs through.
//!
//! # Tokens only
//!
//! A bracket token is the only reference form this parser recognizes. A deep-link URL that points at
//! the same subject is each frontend's own concern: recognizing one is route matching against that
//! frontend's URL grammar, and a connector rewrites such a link to the canonical token before the
//! message reaches this parser (and before it reaches the agent). So this module stays the agent-facing
//! token vocabulary alone, and it is deliberately dependency-light — no URL crate, no regex, only ULID
//! parsing — so it compiles to wasm and drives the frontend across that boundary.

use ulid::Ulid;

use crate::{
    ids::{MemoryId, TurnId},
    mem_ref, turn_ref,
};

/// One span of scanned text: literal prose, or a reference resolved to its typed subject id. A
/// reference variant covers the whole matched token, so re-serialization ([`normalize`]) loses nothing
/// but the token's surface form.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Segment<'a> {
    /// Literal text that carries no reference.
    Prose(&'a str),
    /// A turn reference — a conversation moment.
    Turn(TurnId),
    /// A memory reference — a person, an event, or a place.
    Mem(MemoryId),
}

/// Split `text` into prose, turn references, and memory references, in order — one pass over both token
/// grammars. A malformed body — a `[turn:…]` or `[mem:…]` whose 26 characters are not a valid ULID —
/// stays prose, never a reference. Adjacent prose is coalesced and empty prose spans are dropped, so the
/// segments read back as the original text.
pub fn scan(text: &str) -> Vec<Segment<'_>> {
    let mut segments = Vec::new();
    let bytes = text.as_bytes();
    let mut prose_start = 0;
    let mut i = 0;
    while i < bytes.len() {
        if let Some((token, len)) = token_at(text, i) {
            flush_prose(&mut segments, &text[prose_start..i]);
            segments.push(match token {
                Token::Turn(turn) => Segment::Turn(turn),
                Token::Mem(memory) => Segment::Mem(memory),
            });
            i += len;
            prose_start = i;
            continue;
        }
        i += 1;
    }
    flush_prose(&mut segments, &text[prose_start..]);
    segments
}

/// Rebuild `text` with every reference rendered as its canonical token — turn references through
/// [`crate::turn_ref::construct`] and memory references through [`crate::mem_ref::construct`]. Prose is
/// preserved verbatim, so a message with no references is returned unchanged, and a message already in
/// canonical form round-trips to itself.
pub fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for segment in scan(text) {
        match segment {
            Segment::Prose(prose) => out.push_str(prose),
            Segment::Turn(turn) => out.push_str(&turn_ref::construct(turn)),
            Segment::Mem(memory) => out.push_str(&mem_ref::construct(memory)),
        }
    }
    out
}

/// Every turn id referenced in `text`, in order of appearance — the turn half of the extract-all-ids
/// path (the ambient pass pointing the agent at a cited moment, a connector resolving pasted
/// references).
pub fn extract_turn_ids(text: &str) -> Vec<TurnId> {
    scan(text)
        .into_iter()
        .filter_map(|segment| match segment {
            Segment::Turn(turn) => Some(turn),
            Segment::Prose(_) | Segment::Mem(_) => None,
        })
        .collect()
}

/// Every memory id referenced in `text`, in order of appearance — the memory half of the
/// extract-all-ids path.
pub fn extract_mem_ids(text: &str) -> Vec<MemoryId> {
    scan(text)
        .into_iter()
        .filter_map(|segment| match segment {
            Segment::Mem(memory) => Some(memory),
            Segment::Prose(_) | Segment::Turn(_) => None,
        })
        .collect()
}

/// The `[turn:` opener a turn-reference token starts with — the single source both [`scan`] and
/// [`crate::turn_ref::construct`] read, so the recognized form and the minted form cannot diverge.
pub(crate) const TURN_OPEN: &str = "[turn:";
/// The `[mem:` opener a memory-reference token starts with — shared by [`scan`] and
/// [`crate::mem_ref::construct`].
pub(crate) const MEM_OPEN: &str = "[mem:";
/// A Crockford ULID is exactly 26 characters (all ASCII), so a reference body is a fixed-width slice.
pub(crate) const ULID_LEN: usize = 26;

/// A matched token before it is mapped onto a [`Segment`], so [`token_at`] can report the length of the
/// token it consumed alongside the id it parsed.
enum Token {
    Turn(TurnId),
    Mem(MemoryId),
}

/// The reference token starting at byte `i`, if any, and its byte length — trying the turn grammar, then
/// the memory grammar. `None` when neither matches, so the position stays prose.
fn token_at(text: &str, i: usize) -> Option<(Token, usize)> {
    if let Some(ulid) = bracket_body_at(text, i, TURN_OPEN) {
        return Some((Token::Turn(TurnId(ulid)), TURN_OPEN.len() + ULID_LEN + 1));
    }
    if let Some(ulid) = bracket_body_at(text, i, MEM_OPEN) {
        return Some((Token::Mem(MemoryId(ulid)), MEM_OPEN.len() + ULID_LEN + 1));
    }
    None
}

/// The ULID a bracket token opens at byte `i`, if `text[i..]` is `<open><ulid>]`. `text.get(i..)`
/// returns `None` off a character boundary, so a scan walking every byte cannot slice mid-character and
/// panic; a body that is not a valid ULID yields `None`, so `[turn: whenever]` stays prose.
fn bracket_body_at(text: &str, i: usize, open: &str) -> Option<Ulid> {
    let rest = text.get(i..)?.strip_prefix(open)?;
    // ULID characters are ASCII, so a 26-byte slice is exactly 26 characters.
    let body = rest.get(..ULID_LEN)?;
    if rest.as_bytes().get(ULID_LEN) != Some(&b']') {
        return None;
    }
    Ulid::from_string(body).ok()
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

    /// Ids from raw 128-bit values — proptest generates them without touching the RNG-backed
    /// `generate`, so a round-trip covers the whole id space, including edge values.
    fn turn_id(bits: u128) -> TurnId {
        TurnId(Ulid::from(bits))
    }
    fn memory_id(bits: u128) -> MemoryId {
        MemoryId(Ulid::from(bits))
    }

    #[test]
    fn scan_lifts_both_vocabularies_in_one_pass() {
        let turn = turn_id(1);
        let memory = memory_id(2);
        let text = format!(
            "see {} and {} together",
            turn_ref::construct(turn),
            mem_ref::construct(memory)
        );
        assert_eq!(
            scan(&text),
            vec![
                Segment::Prose("see "),
                Segment::Turn(turn),
                Segment::Prose(" and "),
                Segment::Mem(memory),
                Segment::Prose(" together"),
            ]
        );
        assert_eq!(extract_turn_ids(&text), vec![turn]);
        assert_eq!(extract_mem_ids(&text), vec![memory]);
    }

    #[test]
    fn normalize_canonicalizes_both_kinds_and_round_trips() {
        let turn = turn_id(7);
        let memory = memory_id(9);
        let text = format!(
            "about {} and {} please",
            turn_ref::construct(turn),
            mem_ref::construct(memory)
        );
        assert_eq!(normalize(&text), text);
    }

    #[test]
    fn ordinary_bracketed_prose_is_not_a_reference() {
        for text in [
            "[turn: whenever you like]",
            "[member of the club]",
            "[mem:short]",
            "[turn:short]",
            "a [note] in brackets",
            "turn:left then right",
        ] {
            assert!(
                extract_turn_ids(text).is_empty() && extract_mem_ids(text).is_empty(),
                "false positive on {text:?}"
            );
            assert_eq!(scan(text), vec![Segment::Prose(text)]);
        }
    }

    #[test]
    fn a_malformed_ulid_body_is_rejected() {
        // 26 characters, but `I`, `L`, `O`, and `U` are not in the Crockford alphabet.
        for text in [
            "[turn:IIIIIIIIIIIIIIIIIIIIIIIIII]",
            "[mem:IIIIIIIIIIIIIIIIIIIIIIIIII]",
        ] {
            assert!(extract_turn_ids(text).is_empty() && extract_mem_ids(text).is_empty());
            assert_eq!(scan(text), vec![Segment::Prose(text)]);
        }
    }

    #[test]
    fn multibyte_prose_scans_without_panicking() {
        // The scan walks byte positions, so multibyte characters — an em dash, CJK, an emoji — must
        // never land it on a non-boundary slice (the wasm `unreachable` regression).
        let turn = turn_id(11);
        let memory = memory_id(12);
        let text = format!(
            "さっき決めた — {} 🎉 {} “quotes”",
            turn_ref::construct(turn),
            mem_ref::construct(memory)
        );
        assert_eq!(extract_turn_ids(&text), vec![turn]);
        assert_eq!(extract_mem_ids(&text), vec![memory]);
    }

    proptest! {
        /// Constructing then scanning a token round-trips to the same id, and nothing else.
        #[test]
        fn turn_construct_scan_round_trips(bits in any::<u128>()) {
            let turn = turn_id(bits);
            let token = turn_ref::construct(turn);
            prop_assert_eq!(scan(&token), vec![Segment::Turn(turn)]);
            prop_assert_eq!(extract_turn_ids(&token), vec![turn]);
        }

        #[test]
        fn mem_construct_scan_round_trips(bits in any::<u128>()) {
            let memory = memory_id(bits);
            let token = mem_ref::construct(memory);
            prop_assert_eq!(scan(&token), vec![Segment::Mem(memory)]);
            prop_assert_eq!(extract_mem_ids(&token), vec![memory]);
        }

        /// References of both kinds, interleaved with reference-free prose, extract in order.
        #[test]
        fn interleaved_references_extract_in_order(
            bits in proptest::collection::vec(any::<u128>(), 0..6),
            kinds in proptest::collection::vec(any::<bool>(), 0..6),
        ) {
            let mut text = String::from("start ");
            let mut turns = Vec::new();
            let mut memories = Vec::new();
            for (index, raw) in bits.iter().enumerate() {
                if kinds.get(index).copied().unwrap_or(false) {
                    let turn = turn_id(*raw);
                    turns.push(turn);
                    text.push_str(&turn_ref::construct(turn));
                } else {
                    let memory = memory_id(*raw);
                    memories.push(memory);
                    text.push_str(&mem_ref::construct(memory));
                }
                text.push_str(" then ");
            }
            text.push_str("end");
            prop_assert_eq!(extract_turn_ids(&text), turns);
            prop_assert_eq!(extract_mem_ids(&text), memories);
        }

        /// Scanning arbitrary text — any Unicode, refs or none — never panics (the wasm `unreachable`
        /// regression), and reference-free text reassembles verbatim from its segments (the scan is a
        /// pure split, lossy nowhere).
        #[test]
        fn scan_is_total_and_lossless(text in any::<String>()) {
            let segments = scan(&text);
            if extract_turn_ids(&text).is_empty() && extract_mem_ids(&text).is_empty() {
                let rebuilt: String = segments
                    .iter()
                    .map(|segment| match segment {
                        Segment::Prose(prose) => *prose,
                        _ => unreachable!("no ids extracted"),
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
            let text = format!("{before} {} {after}", turn_ref::construct(turn));
            prop_assert!(extract_turn_ids(&text).contains(&turn));
        }
    }
}
