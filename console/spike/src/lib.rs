//! Spike: does rusqlite reach SQLite under `wasm32-unknown-unknown`?
//!
//! The whole console architecture (a materializing read replica folding the event log through the
//! agent's own rusqlite-backed materializer compiled to WASM) hinges on this one question. This
//! crate answers it in the smallest possible way: open an in-memory connection, create a table,
//! round-trip a row, and hand the result back across the wasm-bindgen boundary. A green build plus
//! a passing run is the go signal for the `zuihitsu-core` carve-out; a red one sends us to the
//! pure-core fallback in `console/PLAN.md`.

use rusqlite::Connection;
use serde::Serialize;
use wasm_bindgen::prelude::*;

/// What the round-trip recovered: the SQLite version string the wasm build reports, and the rows
/// read back out of the in-memory table. Serialized to a JS value so the caller can assert on it.
#[derive(Serialize)]
pub struct SpikeResult {
    sqlite_version: String,
    rows: Vec<Row>,
}

#[derive(Serialize)]
struct Row {
    id: i64,
    label: String,
}

/// Open an in-memory database, write two rows, read them back. Returns the SQLite version and the
/// rows, or the error string if any step fails — the caller renders either verbatim.
#[wasm_bindgen]
pub fn round_trip() -> Result<JsValue, JsValue> {
    run()
        .map_err(|error| JsValue::from_str(&error.to_string()))
        .and_then(|result| {
            serde_wasm_bindgen::to_value(&result)
                .map_err(|error| JsValue::from_str(&error.to_string()))
        })
}

fn run() -> rusqlite::Result<SpikeResult> {
    let connection = Connection::open_in_memory()?;
    let sqlite_version: String =
        connection.query_row("SELECT sqlite_version()", [], |row| row.get(0))?;

    connection.execute_batch(
        "CREATE TABLE memory (id INTEGER PRIMARY KEY, label TEXT NOT NULL);
         INSERT INTO memory (id, label) VALUES (1, 'self'), (2, 'person/dave');",
    )?;

    let mut statement = connection.prepare("SELECT id, label FROM memory ORDER BY id")?;
    let rows = statement
        .query_map([], |row| {
            Ok(Row {
                id: row.get("id")?,
                label: row.get("label")?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    Ok(SpikeResult {
        sqlite_version,
        rows,
    })
}
