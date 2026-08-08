//! Small rusqlite query helpers shared by the SQLite-backed layers (graph, store, vector).
//!
//! [`query_map_into`] and [`query_opt_into`] wrap the prepare-iterate-collect plumbing around a
//! caller-supplied mapping closure that is generic over the error type, so a mapper which unpacks a
//! row (rusqlite's tuple `TryFrom`, via `row.try_into()`) and then does serde/ULID work reads as a
//! single `?`-chain rather than a closure-builds-a-tuple-then-a-second-loop-converts-it dance. The
//! error type must absorb `rusqlite::Error` (`From<rusqlite::Error>`), which each layer's error does.
//!
//! [`sqlite_defaults`] applies the shared per-connection pragma set every SQLite open path uses:
//! `foreign_keys = ON`, `busy_timeout`, `journal_mode = WAL`, `synchronous = NORMAL`. The FKs are
//! load-bearing from the first write, since the graph schema declares `FOREIGN KEY` clauses.

use rusqlite::{Connection, Params, Row, Statement};

/// The busy timeout (milliseconds) every SQLite connection carries, so a momentary lock held by
/// another process surfaces as a wait rather than a spurious `SQLITE_BUSY`. The event log's
/// exclusive advisory lock remains the real one-writer enforcement.
pub const BUSY_TIMEOUT_MS: i64 = 5_000;

/// Apply the shared per-connection SQLite defaults. Order matters:
///
/// 1. `busy_timeout` and `synchronous = NORMAL` first, safe on every connection.
/// 2. `journal_mode = WAL` only when `wal` is set: it is a write, so it is skipped for read-only
///    and in-memory connections.
/// 3. `foreign_keys = ON` last. It is a no-op without FK clauses (the vector store), and the
///    graph schema's clauses make it load-bearing from the first DML.
///
/// Callers pass `wal = false` for a read-only or `:memory:` connection.
pub fn sqlite_defaults(conn: &Connection, wal: bool) -> Result<(), rusqlite::Error> {
    conn.pragma_update(None, "busy_timeout", BUSY_TIMEOUT_MS)?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    if wal {
        conn.pragma_update(None, "journal_mode", "WAL")?;
    }
    conn.pragma_update(None, "foreign_keys", "ON")?;
    Ok(())
}

/// Run `stmt` with `params`, mapping each row through `map` and collecting the results. The mapper
/// may fail with any error that absorbs a `rusqlite::Error`.
pub fn query_map_into<T, E, P, F>(
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
pub fn query_opt_into<T, E, P, F>(
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
