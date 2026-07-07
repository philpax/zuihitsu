use serde::Serialize;
use zuihitsu_core::turn_ref;

/// borrows and carries a typed id; the crossing wants owned strings).
#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum RefSegment<'a> {
    Prose { text: &'a str },
    Ref { id: String },
}

/// Split `text` into prose spans and turn references — the console's pretty projection runs each
/// turn's text through this so a `[turn:<ulid>]` token or a pasted deep-link URL renders as a chip.
/// The parser is `zuihitsu_core::turn_ref::scan`, the same definition the agent's resolver reads, so
/// what the console highlights and what the agent resolves cannot drift.
#[wasm_bindgen(js_name = turnRefScan)]
pub fn turn_ref_scan(text: &str) -> Result<JsValue, JsError> {
    let segments: Vec<RefSegment> = turn_ref::scan(text)
        .into_iter()
        .map(|segment| match segment {
            turn_ref::Segment::Prose(prose) => RefSegment::Prose { text: prose },
            turn_ref::Segment::Ref(turn) => RefSegment::Ref {
                id: turn.0.to_string(),
            },
        })
        .collect();
    to_js(&segments)
}

/// Rebuild `text` with every turn reference rendered as the canonical `[turn:<ulid>]` token — the
/// composer's send-time normalization, so a pasted console URL leaves the console as ref syntax and
/// every downstream consumer sees one form.
#[wasm_bindgen(js_name = turnRefNormalize)]
pub fn turn_ref_normalize(text: &str) -> String {
    turn_ref::normalize(text)
}

/// Every turn id referenced in `text`, in order of appearance — the extract-all-ids path.
#[wasm_bindgen(js_name = turnRefExtract)]
pub fn turn_ref_extract(text: &str) -> Result<JsValue, JsError> {
    let ids: Vec<String> = turn_ref::extract_ids(text)
        .into_iter()
        .map(|turn| turn.0.to_string())
        .collect();
    to_js(&ids)
}

/// The canonical `[turn:<ulid>]` token for a turn id, or an error if `id` is not a ULID — so the
/// console mints citations through the same constructor the agent's `ref` field uses.
#[wasm_bindgen(js_name = turnRefConstruct)]
pub fn turn_ref_construct(id: &str) -> Result<String, JsError> {
    let ulid = Ulid::from_string(id)
        .map_err(|error| JsError::new(&format!("console: parsing the turn id {id:?}: {error}")))?;
    Ok(turn_ref::construct(TurnId(ulid)))
}
