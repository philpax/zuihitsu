use ulid::Ulid;
use wasm_bindgen::prelude::*;
use zuihitsu_core::{ids::MemoryId, mem_ref};

/// The canonical `[mem:<ulid>]` token for a memory id, or an error if `id` is not a ULID — so the
/// console mints references through the same constructor a connector uses. The nav layer calls this to
/// rewrite a pasted State-view deep link (whose handle it has resolved to a memory) to its token.
#[wasm_bindgen(js_name = memRefConstruct)]
pub fn mem_ref_construct(id: &str) -> Result<String, JsError> {
    let ulid = Ulid::from_string(id).map_err(|error| {
        JsError::new(&format!("console: parsing the memory id {id:?}: {error}"))
    })?;
    Ok(mem_ref::construct(MemoryId(ulid)))
}
