//! Hint rendering for the ambient recall pass: the resolved rows the pass surfaces and the terse,
//! ordered text the turn injects as one system message after the inbound.

use std::fmt::Write as _;

use crate::ids::{MemoryId, MemoryName, TurnId};

/// A surviving hit resolved to what the hint renders: the memory's handle and the FTS snippet of its
/// strongest match.
pub(super) struct ResolvedHit {
    pub(super) name: MemoryName,
    pub(super) snippet: String,
}

/// A cited memory reference resolved to what the hint renders: the token's id (as it appears in the
/// message) and the handle it points at, so the agent can decode `[mem:<id>]` to a memory it operates on
/// by handle.
pub(super) struct ResolvedMem {
    pub(super) token: MemoryId,
    pub(super) name: MemoryName,
}

/// Render the hint the turn injects: first one line per resolved memory reference, decoding `[mem:<id>]`
/// to the handle it points at (so a spliced @mention or pasted reference is never opaque), then one line
/// per cited turn token pointing at its `convo.turn` resolver (so an explicit reference is never inert),
/// then one line per shared URL pointing at reading it with `web.markdown` (so a shared link is never
/// inert), then — when lexical hits survive — the "possibly relevant" block, one line per hit naming the
/// handle and its snippet. It sits after the inbound message in the prompt, so it reads as a note about
/// that message. At least one of `mems`, `tokens`, `urls`, or `hits` is non-empty (the caller returns
/// `None` otherwise).
pub(super) fn render(
    mems: &[ResolvedMem],
    tokens: &[TurnId],
    urls: &[String],
    hits: &[ResolvedHit],
) -> String {
    let mut out = String::new();
    for mem in mems {
        if !out.is_empty() {
            out.push('\n');
        }
        // A reference to the reserved `self` memory decodes to the agent itself, which is already in
        // context — so the line names it without the redundant read suggestion.
        if mem.name.is_self() {
            let _ = write!(
                out,
                "[mem:{}] refers to {} — that is you.",
                mem.token.0,
                mem.name.as_str()
            );
        } else {
            let _ = write!(
                out,
                "[mem:{}] refers to {} — read it with memory.get(\"{}\") if useful.",
                mem.token.0,
                mem.name.as_str(),
                mem.name.as_str()
            );
        }
    }
    for token in tokens {
        if !out.is_empty() {
            out.push('\n');
        }
        let _ = write!(
            out,
            "The message above references a recorded moment — read it with convo.turn(\"{}\").",
            token.0
        );
    }
    for url in urls {
        if !out.is_empty() {
            out.push('\n');
        }
        let _ = write!(
            out,
            "The message includes a link — read it with web.markdown(\"{url}\")."
        );
    }
    if !hits.is_empty() {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str("Possibly relevant to the message above — read with memory.get if useful:");
        for hit in hits {
            let snippet = hit.snippet.trim();
            if snippet.is_empty() || snippet == hit.name.as_str() {
                let _ = write!(out, "\n- {}", hit.name.as_str());
            } else {
                let _ = write!(out, "\n- {} — \"{snippet}\"", hit.name.as_str());
            }
        }
    }
    out
}
