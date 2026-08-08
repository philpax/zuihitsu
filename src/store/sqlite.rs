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
    db::{self, query_map_into},
    event::{Event, EventPayload, EventSource},
    ids::Seq,
    store::{Store, StoreError, Subscription, notify},
    time::Timestamp,
};

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
        db::sqlite_defaults(&conn, true).map_err(backend)?;
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
    ///
    /// The log is a WAL database, and SQLite reads a WAL database through its shared-memory index. So
    /// this open succeeds in the two ordinary cases — a cleanly closed log, whose WAL was checkpointed
    /// away, and a log a live writer is holding, whose index is already up — but fails on a log left
    /// hot by a killed process, where recovering the WAL would be a write. The remedy is to boot the
    /// instance read-write once, which recovers and checkpoints it.
    pub fn open_read_only(path: impl AsRef<Path>) -> Result<SqliteStore, StoreError> {
        let conn = Connection::open_with_flags(
            path.as_ref(),
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(backend)?;
        db::sqlite_defaults(&conn, false).map_err(backend)?;
        Ok(SqliteStore {
            conn,
            subscribers: Vec::new(),
            _lock: None,
        })
    }

    /// Open an ephemeral in-memory log. Used by tests; WAL and locking are not applicable here.
    pub fn open_in_memory() -> Result<SqliteStore, StoreError> {
        let conn = Connection::open_in_memory().map_err(backend)?;
        db::sqlite_defaults(&conn, false).map_err(backend)?;
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
             ) STRICT;
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

#[cfg(test)]
mod tests {
    //! The SQLite backend's own storage-level properties: the STRICT `events` table (wrong-typed
    //! writes are rejected, not coerced) and the shared per-connection pragma set on every open path.

    use crate::store::SqliteStore;

    /// A fresh log's `events` table is STRICT: the DDL text carries the keyword, and a REAL value
    /// written into an INTEGER column is rejected with a constraint error — a non-STRICT table
    /// would coerce the value by affinity instead. The latter is what distinguishes STRICT from
    /// affinity-only rejection (a wrong-typed TEXT-vs-INTEGER insert is already refused by
    /// affinity on a non-STRICT table).
    #[test]
    fn events_is_strict_in_a_fresh_log() {
        let store = SqliteStore::open_in_memory().unwrap();
        let ddl: String = store
            .conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'events'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            ddl.contains("STRICT"),
            "the events table must be declared STRICT, got: {ddl}"
        );

        // A REAL (3.5) into the INTEGER `seq` column: STRICT tables reject it with a
        // "datatype mismatch" constraint violation; a non-STRICT table would store 3 with its
        // REAL affinity. The error surfaces as a constraint violation, not a coercion.
        let error = store
            .conn
            .execute(
                "INSERT INTO events (seq, recorded_at, type, target_id, version, source, payload)
                 VALUES (3.5, 1, 'MemoryDeleted', NULL, 1, 'Agent', '{}')",
                [],
            )
            .unwrap_err();
        let message = error.to_string();
        assert!(
            message.contains("datatype mismatch"),
            "a REAL into an INTEGER column of a STRICT table must be rejected with \
             'datatype mismatch', got: {message}"
        );
    }

    /// The shared pragma set is on from the first open: `foreign_keys` and `synchronous` are
    /// per-connection, and a writable file-backed log is in WAL mode with a busy timeout. The
    /// in-memory log skips WAL (meaningless there) but keeps the rest. (The `synchronous` value is
    /// asserted on the file-backed log, where NORMAL is meaningful; SQLite reports the in-memory
    /// journal's effective value, which is not the pragma as written.)
    #[test]
    fn a_fresh_log_carries_the_shared_pragma_defaults() {
        let store = SqliteStore::open_in_memory().unwrap();
        let foreign_keys: i64 = store
            .conn
            .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
            .unwrap();
        assert_eq!(foreign_keys, 1, "foreign_keys must be ON");
        let busy_timeout: i64 = store
            .conn
            .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
            .unwrap();
        assert_eq!(busy_timeout, 5_000, "busy_timeout must be 5000ms");
        // journal_mode on a `:memory:` log reads back as "memory" — the WAL pragma is skipped there.
        let journal_mode: String = store
            .conn
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .unwrap();
        assert_eq!(
            journal_mode, "memory",
            "an in-memory log must not be switched to WAL"
        );
    }

    /// The shared pragma set also lands on a writable file-backed log — the production shape — with
    /// WAL active and `synchronous = NORMAL`, so a killed process's `-wal`/`-shm` siblings can be
    /// recovered on the next open.
    #[test]
    fn a_file_log_switches_to_wal() {
        let path = std::env::temp_dir().join(format!(
            "zuihitsu-wal-{}.sqlite",
            crate::ids::MemoryId::generate().0
        ));
        {
            let store = SqliteStore::open(&path).unwrap();
            let journal_mode: String = store
                .conn
                .query_row("PRAGMA journal_mode", [], |row| row.get(0))
                .unwrap();
            assert_eq!(journal_mode, "wal", "a file-backed log must be in WAL mode");
            let foreign_keys: i64 = store
                .conn
                .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
                .unwrap();
            assert_eq!(foreign_keys, 1);
            let synchronous: i64 = store
                .conn
                .query_row("PRAGMA synchronous", [], |row| row.get(0))
                .unwrap();
            assert_eq!(synchronous, 1, "synchronous must be NORMAL (1)");
        }
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
        let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
    }
}
