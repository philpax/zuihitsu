use ulid::Ulid;
use wasm_bindgen::prelude::*;
use zuihitsu_core::{ids::TurnId, turn_ref};

/// The canonical `[turn:<ulid>]` token for a turn id, or an error if `id` is not a ULID — so the console
/// mints citations through the same constructor the agent's `ref` field uses. The nav layer calls this
/// to rewrite a pasted conversation deep link (`…?turn=<id>`) to its token.
#[wasm_bindgen(js_name = turnRefConstruct)]
pub fn turn_ref_construct(id: &str) -> Result<String, JsError> {
    let ulid = Ulid::from_string(id)
        .map_err(|error| JsError::new(&format!("console: parsing the turn id {id:?}: {error}")))?;
    Ok(turn_ref::construct(TurnId(ulid)))
}
