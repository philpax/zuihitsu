//! The one combined reference scan the console's transcript projection reads. A turn's text carries
//! two token vocabularies — `[turn:<ulid>]` (a moment) and `[mem:<ulid>]` (a memory) — and the remark
//! pass wants both lifted out in a single wasm call, dispatching only on the returned `kind` and never
//! inspecting token syntax itself. This composes the two core parsers ([`zuihitsu_core::turn_ref`] and
//! [`zuihitsu_core::mem_ref`]) into one segment stream; the per-vocabulary exports (`turn_ref`,
//! `mem_ref`) remain for the composer's per-kind normalization and construction.
//!
//! Console URL awareness lives entirely on the frontend (route matching in the nav layer), not here:
//! this scan recognizes tokens only. A `?turn=` deep link is still folded in wherever the core
//! `turn_ref` parser already recognizes one, since composing that parser costs nothing and keeps the
//! existing turn behavior; memory deep links, which route by handle, are matched and resolved in
//! TypeScript.

use wasm_bindgen::prelude::*;
use zuihitsu_core::{mem_ref, turn_ref};

use crate::types::{RefSegment, RefSegmentList};

/// Split `text` into prose, turn references, and memory references in one pass — the transcript's
/// pretty projection runs each turn's text through this so both `[turn:<ulid>]` and `[mem:<ulid>]`
/// tokens render as chips from a single call. Turn references are lifted first (the core `turn_ref`
/// parser, which also folds in a `?turn=` deep link), then memory references within each remaining
/// prose span (the core `mem_ref` parser, tokens only); the two token vocabularies never overlap.
#[wasm_bindgen(js_name = refScan)]
pub fn ref_scan(text: &str) -> RefSegmentList {
    let mut segments = Vec::new();
    for turn_segment in turn_ref::scan(text) {
        match turn_segment {
            turn_ref::Segment::Ref(turn) => segments.push(RefSegment::Turn {
                id: turn.0.to_string(),
            }),
            turn_ref::Segment::Prose(prose) => {
                for mem_segment in mem_ref::scan(prose) {
                    match mem_segment {
                        mem_ref::Segment::Ref(memory) => segments.push(RefSegment::Mem {
                            id: memory.0.to_string(),
                        }),
                        mem_ref::Segment::Prose(text) => segments.push(RefSegment::Prose {
                            text: text.to_string(),
                        }),
                    }
                }
            }
        }
    }
    RefSegmentList(segments)
}
