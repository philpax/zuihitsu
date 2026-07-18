use serde::Serialize;
use ulid::Ulid;
use wasm_bindgen::prelude::*;
use zuihitsu_core::{ids::MemoryId, mem_ref};

use crate::to_js;

/// One span of scanned memory-reference text, crossing to the console: literal prose, or a reference
/// resolved to its memory's ULID. Mirrors the `turn_ref` wire shape (`RefSegment` there); the crossing
/// wants owned strings.
#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum RefSegment<'a> {
    Prose { text: &'a str },
    Ref { id: String },
}

/// Split `text` into prose spans and memory references — the console's pretty projection runs each
/// turn's text through this so a `[mem:<ulid>]` token renders as a chip. The parser is
/// `zuihitsu_core::mem_ref::scan`, the same definition a connector splices tokens under, so what the
/// console highlights and what a connector writes cannot drift. Core recognizes no URL form here (a
/// memory's deep link routes by handle, not id); the graph-aware URL half is `normalizeMemRefs`.
#[wasm_bindgen(js_name = memRefScan)]
pub fn mem_ref_scan(text: &str) -> Result<JsValue, JsError> {
    let segments: Vec<RefSegment> = mem_ref::scan(text)
        .into_iter()
        .map(|segment| match segment {
            mem_ref::Segment::Prose(prose) => RefSegment::Prose { text: prose },
            mem_ref::Segment::Ref(memory) => RefSegment::Ref {
                id: memory.0.to_string(),
            },
        })
        .collect();
    to_js(&segments)
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
pub fn mem_ref_extract(text: &str) -> Result<JsValue, JsError> {
    let ids: Vec<String> = mem_ref::extract_ids(text)
        .into_iter()
        .map(|memory| memory.0.to_string())
        .collect();
    to_js(&ids)
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
