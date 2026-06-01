//! The SQLite-backed event store: durable, append-only, WAL-mode. One `events` table, written
//! once and never modified; if everything else is lost, the system rebuilds from this (spec
//! §Storage). The per-process subscriber set is shared with the in-memory backend via `notify`.

use std::path::Path;
use std::sync::mpsc::{Sender, channel};

use rusqlite::{Connection, params};

use crate::event::{Event, EventPayload};
use crate::ids::{Seq, Timestamp};

use super::{Store, StoreError, Subscription, notify};

pub struct SqliteStore {
    conn: Connection,
    subscribers: Vec<Sender<Event>>,
}

impl SqliteStore {
    /// Open (creating if absent) a file-backed log in WAL mode.
    pub fn open(path: impl AsRef<Path>) -> Result<SqliteStore, StoreError> {
        let conn = Connection::open(path).map_err(backend)?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(backend)?;
        Self::init(conn)
    }

    /// Open an ephemeral in-memory log. Used by tests; WAL is not applicable here.
    pub fn open_in_memory() -> Result<SqliteStore, StoreError> {
        let conn = Connection::open_in_memory().map_err(backend)?;
        Self::init(conn)
    }

    fn init(conn: Connection) -> Result<SqliteStore, StoreError> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS events (
                 seq         INTEGER PRIMARY KEY,
                 recorded_at INTEGER NOT NULL,
                 type        TEXT    NOT NULL,
                 target_id   TEXT,
                 version     INTEGER NOT NULL,
                 payload     TEXT    NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_events_target ON events(target_id);",
        )
        .map_err(backend)?;
        Ok(SqliteStore {
            conn,
            subscribers: Vec::new(),
        })
    }
}

impl Store for SqliteStore {
    fn append(
        &mut self,
        recorded_at: Timestamp,
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
                "INSERT INTO events (seq, recorded_at, type, target_id, version, payload)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    seq,
                    recorded_at.as_millis(),
                    payload.kind(),
                    payload.target_id(),
                    payload.version(),
                    json,
                ],
            )
            .map_err(backend)?;
            committed.push(Event {
                seq: Seq(seq as u64),
                recorded_at,
                payload,
            });
        }
        tx.commit().map_err(backend)?;

        notify(&mut self.subscribers, &committed);
        Ok(committed)
    }

    fn read_from(&self, from: Seq) -> Result<Vec<Event>, StoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT seq, recorded_at, payload FROM events WHERE seq >= ?1 ORDER BY seq")
            .map_err(backend)?;
        let rows = stmt
            .query_map(params![from.0 as i64], |row| {
                let seq: i64 = row.get(0)?;
                let recorded_at: i64 = row.get(1)?;
                let payload: String = row.get(2)?;
                Ok((seq, recorded_at, payload))
            })
            .map_err(backend)?;

        let mut events = Vec::new();
        for row in rows {
            let (seq, recorded_at, payload) = row.map_err(backend)?;
            events.push(Event {
                seq: Seq(seq as u64),
                recorded_at: Timestamp::from_millis(recorded_at),
                payload: serde_json::from_str(&payload)?,
            });
        }
        Ok(events)
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

    fn subscribe(&mut self) -> Subscription {
        let (sender, receiver) = channel();
        self.subscribers.push(sender);
        receiver
    }
}

fn backend(error: rusqlite::Error) -> StoreError {
    StoreError::Backend(error.to_string())
}
