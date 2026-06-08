//! Small rusqlite query helpers shared by the SQLite-backed layers (graph, store, vector).
//!
//! [`query_map_into`] and [`query_opt_into`] wrap the prepare-iterate-collect plumbing around a
//! caller-supplied mapping closure that is generic over the error type, so a mapper which unpacks a
//! row (rusqlite's tuple `TryFrom`, via `row.try_into()`) and then does serde/ULID work reads as a
//! single `?`-chain rather than a closure-builds-a-tuple-then-a-second-loop-converts-it dance. The
//! error type must absorb `rusqlite::Error` (`From<rusqlite::Error>`), which each layer's error does.

use rusqlite::{Params, Row, Statement};

/// Run `stmt` with `params`, mapping each row through `map` and collecting the results. The mapper
/// may fail with any error that absorbs a `rusqlite::Error`.
pub(crate) fn query_map_into<T, E, P, F>(
    mut stmt: Statement<'_>,
    params: P,
    mut map: F,
) -> Result<Vec<T>, E>
where
    P: Params,
    F: FnMut(&Row<'_>) -> Result<T, E>,
    E: From<rusqlite::Error>,
{
    let mut rows = stmt.query(params)?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        out.push(map(row)?);
    }
    Ok(out)
}

/// As [`query_map_into`], for a query expected to yield at most one row: maps the first row, or
/// returns `None` when there is none.
pub(crate) fn query_opt_into<T, E, P, F>(
    mut stmt: Statement<'_>,
    params: P,
    map: F,
) -> Result<Option<T>, E>
where
    P: Params,
    F: FnOnce(&Row<'_>) -> Result<T, E>,
    E: From<rusqlite::Error>,
{
    let mut rows = stmt.query(params)?;
    match rows.next()? {
        Some(row) => Ok(Some(map(row)?)),
        None => Ok(None),
    }
}
