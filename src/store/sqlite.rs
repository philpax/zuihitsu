//! The SQLite-backed event store: durable, append-only, WAL-mode. One `events` table, written
//! once and never modified; if everything else is lost, the system rebuilds from this (spec
//! §Storage). The per-process subscriber set is shared with the in-memory backend via `notify`.

use std::{
    fs::File,
    path::Path,
    sync::mpsc::{Sender, channel},
};

use fs2::FileExt;
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};

use crate::{
    db::query_map_into,
    event::{Event, EventPayload, EventSource},
    ids::Seq,
    time::Timestamp,
};

use crate::store::{Store, StoreError, Subscription, notify};

pub struct SqliteStore {
    conn: Connection,
    subscribers: Vec<Sender<Event>>,
    // Held for the store's lifetime: an exclusive advisory lock enforcing one log, one writer
    // (spec principle 10). `None` for in-memory logs, which can't be shared. Released on drop.
    _lock: Option<File>,
}

impl SqliteStore {
    /// Open (creating if absent) a file-backed log in WAL mode, taking an exclusive lock on it.
    /// Fails if another writer already holds the log — the runtime enforcement of one-writer.
    pub fn open(path: impl AsRef<Path>) -> Result<SqliteStore, StoreError> {
        let path = path.as_ref();
        let conn = Connection::open(path).map_err(backend)?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(backend)?;
        let lock =
            File::open(path).map_err(|e| StoreError::Backend(format!("open log lock: {e}")))?;
        lock.try_lock_exclusive().map_err(|_| {
            StoreError::Backend(format!(
                "event log {} is already open by another writer",
                path.display()
            ))
        })?;
        Self::init(conn, Some(lock))
    }

    /// Open a file-backed log read-only, taking no lock — safe to read while another process holds the
    /// write lock (an operator inspecting a running agent's log). The connection is read-only and the
    /// tables it reads already exist, so no `CREATE` runs; an append against it would error.
    pub fn open_read_only(path: impl AsRef<Path>) -> Result<SqliteStore, StoreError> {
        let conn = Connection::open_with_flags(
            path.as_ref(),
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(backend)?;
        Ok(SqliteStore {
            conn,
            subscribers: Vec::new(),
            _lock: None,
        })
    }

    /// Open an ephemeral in-memory log. Used by tests; WAL and locking are not applicable here.
    pub fn open_in_memory() -> Result<SqliteStore, StoreError> {
        let conn = Connection::open_in_memory().map_err(backend)?;
        Self::init(conn, None)
    }

    fn init(conn: Connection, lock: Option<File>) -> Result<SqliteStore, StoreError> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS events (
                 seq         INTEGER PRIMARY KEY,
                 recorded_at INTEGER NOT NULL,
                 type        TEXT    NOT NULL,
                 target_id   TEXT,
                 version     INTEGER NOT NULL,
                 source      TEXT    NOT NULL DEFAULT 'Agent',
                 payload     TEXT    NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_events_target ON events(target_id);",
        )
        .map_err(backend)?;
        Self::migrate_source_column(&conn)?;
        Ok(SqliteStore {
            conn,
            subscribers: Vec::new(),
            _lock: lock,
        })
    }

    /// Add the envelope `source` column to a log written before it existed. A fresh table already
    /// carries the column from `CREATE`; a pre-source table has every other column but not this one,
    /// so a plain `ADD COLUMN` back-fills it with the [`EventSource::Agent`] default — the same
    /// fallback the serde default gives an unstamped event on the wire (spec §Schema evolution).
    fn migrate_source_column(conn: &Connection) -> Result<(), StoreError> {
        let has_source = conn
            .prepare("SELECT 1 FROM pragma_table_info('events') WHERE name = 'source'")
            .map_err(backend)?
            .exists([])
            .map_err(backend)?;
        if !has_source {
            conn.execute_batch(
                "ALTER TABLE events ADD COLUMN source TEXT NOT NULL DEFAULT 'Agent'",
            )
            .map_err(backend)?;
        }
        Ok(())
    }
}

impl Store for SqliteStore {
    fn append(
        &mut self,
        recorded_at: Timestamp,
        source: EventSource,
        payloads: Vec<EventPayload>,
    ) -> Result<Vec<Event>, StoreError> {
        let tx = self.conn.transaction().map_err(backend)?;
        let mut seq: i64 = tx
            .query_row("SELECT COALESCE(MAX(seq), 0) FROM events", [], |row| {
                row.get(0)
            })
            .map_err(backend)?;

        let mut committed = Vec::with_capacity(payloads.len());
        for payload in payloads {
            seq += 1;
            let json = serde_json::to_string(&payload)?;
            tx.execute(
                "INSERT INTO events (seq, recorded_at, type, target_id, version, source, payload)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    seq,
                    recorded_at.as_millisecond(),
                    payload.kind(),
                    payload.target_id(),
                    payload.version(),
                    source.as_str(),
                    json,
                ],
            )
            .map_err(backend)?;
            committed.push(Event {
                seq: Seq(seq as u64),
                recorded_at,
                source: source.clone(),
                payload,
            });
        }
        tx.commit().map_err(backend)?;

        notify(&mut self.subscribers, &committed);
        Ok(committed)
    }

    fn read_from(&self, from: Seq) -> Result<Vec<Event>, StoreError> {
        let stmt = self.conn.prepare(
            "SELECT seq, recorded_at, source, payload FROM events WHERE seq >= ?1 ORDER BY seq",
        )?;
        query_map_into(stmt, params![from.0 as i64], |row| {
            let seq: i64 = row.get("seq")?;
            let recorded_at: i64 = row.get("recorded_at")?;
            let source: String = row.get("source")?;
            let payload: String = row.get("payload")?;
            Ok(Event {
                seq: Seq(seq as u64),
                recorded_at: Timestamp::try_from_millis(recorded_at).ok_or_else(|| {
                    StoreError::Backend(format!(
                        "recorded_at {recorded_at} milliseconds since the Unix epoch is outside \
                         the representable range"
                    ))
                })?,
                // A back-filled or legacy row carries the `Agent` default (the column default and the
                // serde fallback agree), so an unrecognised label falling back to it stays faithful.
                source: source.parse().unwrap_or_default(),
                payload: serde_json::from_str(&payload)?,
            })
        })
    }

    fn head(&self) -> Result<Seq, StoreError> {
        let seq: i64 = self
            .conn
            .query_row("SELECT COALESCE(MAX(seq), 0) FROM events", [], |row| {
                row.get(0)
            })
            .map_err(backend)?;
        Ok(Seq(seq as u64))
    }

    fn recorded_at(&self, seq: Seq) -> Result<Option<Timestamp>, StoreError> {
        let recorded_at: Option<i64> = self
            .conn
            .query_row(
                "SELECT recorded_at FROM events WHERE seq = ?1",
                params![seq.0 as i64],
                |row| row.get(0),
            )
            .optional()
            .map_err(backend)?;
        Ok(recorded_at.map(Timestamp::from_millis))
    }

    fn truncate_to(&mut self, to: Seq) -> Result<u64, StoreError> {
        let removed = self
            .conn
            .execute("DELETE FROM events WHERE seq > ?1", params![to.0 as i64])
            .map_err(backend)?;
        Ok(removed as u64)
    }

    fn subscribe(&mut self) -> Subscription {
        let (sender, receiver) = channel();
        self.subscribers.push(sender);
        receiver
    }
}

fn backend(error: rusqlite::Error) -> StoreError {
    StoreError::Backend(error.to_string())
}
