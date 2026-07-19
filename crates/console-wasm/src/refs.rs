//! The unified reference parser the console reads across the wasm boundary. A turn's text carries two
//! token vocabularies — `[turn:<ulid>]` (a moment) and `[mem:<ulid>]` (a memory) — and both the
//! transcript's pretty projection and the composer's send-time normalization want them handled in one
//! pass. These thin exports map [`zuihitsu_core::message_refs`] onto the boundary; the remark pass
//! dispatches only on the returned `kind` and never inspects token syntax itself.
//!
//! URL awareness lives entirely on the frontend (route matching in the nav layer), not here: these read
//! tokens only. A deep-link URL — a conversation link carrying `?turn=`, a memory's State-view link
//! routing by handle — is recognized by the frontend's own route matching and rewritten to a canonical
//! token before it reaches these functions.

use wasm_bindgen::prelude::*;
use zuihitsu_core::message_refs::{self, Segment};

use crate::types::{RefSegment, RefSegmentList};

/// Split `text` into prose, turn references, and memory references in one pass — the transcript's pretty
/// projection runs each turn's text through this so both `[turn:<ulid>]` and `[mem:<ulid>]` tokens
/// render as chips from a single call, the caller dispatching only on `kind`.
#[wasm_bindgen(js_name = refScan)]
pub fn ref_scan(text: &str) -> RefSegmentList {
    let segments = message_refs::scan(text)
        .into_iter()
        .map(|segment| match segment {
            Segment::Prose(prose) => RefSegment::Prose {
                text: prose.to_string(),
            },
            Segment::Turn(turn) => RefSegment::Turn {
                id: turn.0.to_string(),
            },
            Segment::Mem(memory) => RefSegment::Mem {
                id: memory.0.to_string(),
            },
        })
        .collect();
    RefSegmentList(segments)
}

/// Rebuild `text` with every reference token — turn or memory — collapsed to its canonical form. The
/// composer's send-time normalization runs this after the nav layer has rewritten any deep-link URL to a
/// token, so a message that leaves the console carries only canonical token syntax.
#[wasm_bindgen(js_name = refNormalize)]
pub fn ref_normalize(text: &str) -> String {
    message_refs::normalize(text)
}
