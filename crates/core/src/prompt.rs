//! Typed sections of the assembled system prompt.
//!
//! The system prompt is a single frozen string built from a fixed sequence of parts — the scaffold,
//! the agent's identity, the API reference, the runtime vocabulary, the contextual brief, and the
//! declared current time. Recording *where* each part lands lets the console break a recorded prompt
//! back into its parts without re-deriving the boundaries heuristically.
//!
//! [`AssembledPrompt`] is the builder that carries this guarantee by construction: every byte it
//! emits is attributed to exactly one [`PromptSectionKind`], so the recorded spans tile the final
//! string with no gaps, overlaps, or out-of-bounds ranges. There is no separate validation step —
//! the only way to grow the string is through [`AssembledPrompt::push`], which records the span it
//! just appended, so a malformed set of spans is unrepresentable.

use serde::{Deserialize, Serialize};

/// One part of the assembled system prompt, in emission order. The kinds are the fixed structural
/// parts a prompt is composed from; a given prompt may omit any of the conditional ones (identity,
/// API reference, vocabulary, or brief) when their source is empty.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum PromptSectionKind {
    /// The durable scaffold template — how the agent acts.
    Scaffold,
    /// The agent's identity, drawn verbatim from `self`'s content entries.
    Identity,
    /// The build-derived Lua API reference, including the method-notation legend.
    ApiReference,
    /// The runtime tag vocabulary and registered relations.
    Vocabulary,
    /// The session's frozen contextual brief.
    Brief,
    /// The declared session start time.
    CurrentTime,
}

/// The byte span a section occupies in the assembled prompt, as a half-open `[start, end)` range
/// over the prompt's UTF-8 bytes. Slicing the prompt text by this range yields the section's entire
/// contribution, including any leading separator and header.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PromptSectionSpan {
    /// Which structural part this span covers.
    pub kind: PromptSectionKind,
    /// The inclusive start byte offset into the prompt text.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub start: u32,
    /// The exclusive end byte offset into the prompt text.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub end: u32,
}

/// A span-recording builder for the system prompt. Each [`push`](Self::push) appends to the growing
/// string and records the byte span it covered, so the accumulated spans tile the text exactly:
/// contiguous from zero to the string's length, non-overlapping, and always on UTF-8 boundaries. The
/// builder is the proof of the tiling invariant — there is no other way to grow the string, so the
/// spans cannot drift from what they describe.
#[derive(Clone, Debug, Default)]
pub struct AssembledPrompt {
    text: String,
    sections: Vec<PromptSectionSpan>,
}

impl AssembledPrompt {
    /// Append `text` under `kind`, recording the span it covers. An empty `text` records nothing, so
    /// an absent conditional section leaves no span. Consecutive pushes of the same kind extend the
    /// previous span rather than recording a second one, so a section assembled from several pushes
    /// (a separator, a header, and a body) reads back as one contiguous span.
    pub fn push(&mut self, kind: PromptSectionKind, text: &str) {
        if text.is_empty() {
            return;
        }
        // Section spans index the prompt with `u32` offsets, so the text must stay under 4 GiB. A
        // real system prompt is kilobytes; this ceiling only guards against a pathological caller.
        debug_assert!(
            self.text.len() + text.len() <= u32::MAX as usize,
            "prompt text exceeds the u32 span-offset ceiling"
        );
        let start = self.text.len() as u32;
        self.text.push_str(text);
        let end = self.text.len() as u32;
        if let Some(last) = self.sections.last_mut()
            && last.kind == kind
        {
            last.end = end;
            return;
        }
        self.sections.push(PromptSectionSpan { kind, start, end });
    }

    /// The assembled prompt text.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The recorded section spans, in emission order.
    pub fn sections(&self) -> &[PromptSectionSpan] {
        &self.sections
    }

    /// Consume the builder, yielding the text and its section spans.
    pub fn into_parts(self) -> (String, Vec<PromptSectionSpan>) {
        (self.text, self.sections)
    }
}

#[cfg(test)]
mod tests {
    use super::{AssembledPrompt, PromptSectionKind, PromptSectionSpan};
    use proptest::prelude::*;

    fn kind_of(byte: u8) -> PromptSectionKind {
        match byte % 6 {
            0 => PromptSectionKind::Scaffold,
            1 => PromptSectionKind::Identity,
            2 => PromptSectionKind::ApiReference,
            3 => PromptSectionKind::Vocabulary,
            4 => PromptSectionKind::Brief,
            _ => PromptSectionKind::CurrentTime,
        }
    }

    proptest! {
        /// context-debugger.AC1.1: for arbitrary section inputs — including empty texts and repeated
        /// kinds — the recorded spans tile the text exactly: contiguous from zero to the string's
        /// length, non-overlapping, and each on valid UTF-8 boundaries.
        #[test]
        fn spans_tile_the_text_exactly(pushes in prop::collection::vec((any::<u8>(), any::<String>()), 0..24)) {
            let mut prompt = AssembledPrompt::default();
            for (byte, text) in &pushes {
                prompt.push(kind_of(*byte), text);
            }

            let spans = prompt.sections();
            let text = prompt.text();

            let mut cursor = 0u32;
            for span in spans {
                prop_assert!(span.start < span.end, "an empty span was recorded");
                prop_assert_eq!(span.start, cursor, "a gap or overlap precedes this span");
                prop_assert!(
                    text.get(span.start as usize..span.end as usize).is_some(),
                    "the span lands off a UTF-8 boundary",
                );
                cursor = span.end;
            }
            prop_assert_eq!(cursor as usize, text.len(), "the spans do not reach the end of the text");
        }
    }

    #[test]
    fn recorded_slices_equal_the_pushed_texts() {
        let mut prompt = AssembledPrompt::default();
        prompt.push(PromptSectionKind::Scaffold, "how you act");
        prompt.push(PromptSectionKind::Brief, "\n\n# Brief\n\n");
        prompt.push(PromptSectionKind::Brief, "what you know");
        prompt.push(PromptSectionKind::CurrentTime, "\n\nnow.");

        let (text, sections) = prompt.into_parts();
        let slice =
            |span: &PromptSectionSpan| text[span.start as usize..span.end as usize].to_owned();

        assert_eq!(
            sections.len(),
            3,
            "the two Brief pushes merge into one span"
        );
        assert_eq!(sections[0].kind, PromptSectionKind::Scaffold);
        assert_eq!(slice(&sections[0]), "how you act");
        assert_eq!(sections[1].kind, PromptSectionKind::Brief);
        assert_eq!(slice(&sections[1]), "\n\n# Brief\n\nwhat you know");
        assert_eq!(sections[2].kind, PromptSectionKind::CurrentTime);
        assert_eq!(slice(&sections[2]), "\n\nnow.");
    }

    #[test]
    fn an_empty_push_records_no_span() {
        let mut prompt = AssembledPrompt::default();
        prompt.push(PromptSectionKind::Scaffold, "body");
        prompt.push(PromptSectionKind::Identity, "");
        prompt.push(PromptSectionKind::Brief, "more");

        assert_eq!(prompt.text(), "bodymore");
        let kinds: Vec<_> = prompt.sections().iter().map(|span| span.kind).collect();
        assert_eq!(
            kinds,
            vec![PromptSectionKind::Scaffold, PromptSectionKind::Brief]
        );
    }
}
