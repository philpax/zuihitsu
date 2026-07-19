use ulid::Ulid;
use wasm_bindgen::prelude::*;
use zuihitsu_core::{ids::MemoryId, mem_ref};

use crate::types::{TurnRefSegment, TurnRefSegmentList};

/// Split `text` into prose spans and memory references — the console's pretty projection runs each
/// turn's text through this so a `[mem:<ulid>]` token renders as a chip. The parser is
/// `zuihitsu_core::mem_ref::scan`, the same definition a connector splices tokens under, so what the
/// console highlights and what a connector writes cannot drift. Core recognizes no URL form here (a
/// memory's deep link routes by handle, not id); the graph-aware URL half is `normalizeMemRefs`. The
/// segment shape is the shared single-vocabulary [`TurnRefSegment`], identical for both scans.
#[wasm_bindgen(js_name = memRefScan)]
pub fn mem_ref_scan(text: &str) -> TurnRefSegmentList {
    let segments: Vec<TurnRefSegment> = mem_ref::scan(text)
        .into_iter()
        .map(|segment| match segment {
            mem_ref::Segment::Prose(prose) => TurnRefSegment::Prose {
                text: prose.to_string(),
            },
            mem_ref::Segment::Ref(memory) => TurnRefSegment::Ref {
                id: memory.0.to_string(),
            },
        })
        .collect();
    TurnRefSegmentList(segments)
}

/// Rebuild `text` with every bracket memory reference rendered as the canonical `[mem:<ulid>]` token.
/// A pasted token is already canonical, so this is the token-only half; the composer's graph-aware
/// normalization (`normalizeMemRefs` on `Replica`) additionally collapses a pasted state-view URL.
#[wasm_bindgen(js_name = memRefNormalize)]
pub fn mem_ref_normalize(text: &str) -> String {
    mem_ref::normalize(text)
}

/// Every memory id referenced in `text`, in order of appearance — the extract-all-ids path.
#[wasm_bindgen(js_name = memRefExtract)]
pub fn mem_ref_extract(text: &str) -> Vec<String> {
    mem_ref::extract_ids(text)
        .into_iter()
        .map(|memory| memory.0.to_string())
        .collect()
}

/// The canonical `[mem:<ulid>]` token for a memory id, or an error if `id` is not a ULID — so the
/// console mints references through the same constructor a connector uses.
#[wasm_bindgen(js_name = memRefConstruct)]
pub fn mem_ref_construct(id: &str) -> Result<String, JsError> {
    let ulid = Ulid::from_string(id).map_err(|error| {
        JsError::new(&format!("console: parsing the memory id {id:?}: {error}"))
    })?;
    Ok(mem_ref::construct(MemoryId(ulid)))
}
