//! Turn ID mapping: Discord message IDs to zuihitsu `TurnId`s for `[turn:<id>]` token injection.
//!
//! Maps both agent responses and participant messages to their zuihitsu turn IDs. When a Discord
//! user replies to a mapped message, the connector injects a `[turn:<id>]` token into the message
//! text before forwarding to the platform API.

use std::{path::PathBuf, time::Duration};

use rusqlite::{Connection, OptionalExtension};
use serenity::model::id::MessageId;
use zuihitsu_core::{ids::TurnId, turn_ref};

/// A mapping from Discord message IDs to zuihitsu turn IDs, backed by SQLite.
///
/// When created with a path, the database persists to disk — a connector restart recovers the
/// full mapping. When created without a path, it lives in memory and is lost on restart.
///
/// The `record` and `get`/`inject_turn_ref` methods are synchronous because SQLite operations on
/// a single-file database are fast (sub-millisecond for a point lookup). The caller holds a
/// `Mutex` around the `TurnMap` so there is no concurrent access.
pub struct TurnMap {
    conn: Connection,
}

impl TurnMap {
    /// Open a persistent turn map at `path`, creating the database and schema if it doesn't exist.
    pub fn open(path: &PathBuf) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        Self::init(&conn)?;
        Ok(TurnMap { conn })
    }

    /// Create an in-memory turn map (lost on restart). Used in tests.
    #[cfg(test)]
    pub fn in_memory() -> rusqlite::Result<Self> {
        let conn = Connection::open_in_memory()?;
        Self::init(&conn)?;
        Ok(TurnMap { conn })
    }

    fn init(conn: &Connection) -> rusqlite::Result<()> {
        // The identity sync shares this file through its own connection, so a writer waits for the
        // other's brief write lock rather than failing `SQLITE_BUSY`.
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS turn_map (
                message_id INTEGER PRIMARY KEY,
                turn_id    TEXT NOT NULL
            );",
        )
    }

    /// Record a mapping from a Discord message ID to a zuihitsu turn ID.
    pub fn record(&mut self, message_id: MessageId, turn_id: TurnId) {
        let _ = self.conn.execute(
            "INSERT OR REPLACE INTO turn_map (message_id, turn_id) VALUES (?1, ?2)",
            rusqlite::params![message_id.get() as i64, turn_id.0.to_string()],
        );
    }

    /// Look up the turn ID for a Discord message, if mapped.
    pub fn get(&self, message_id: &MessageId) -> Option<TurnId> {
        self.conn
            .query_row(
                "SELECT turn_id FROM turn_map WHERE message_id = ?1",
                rusqlite::params![message_id.get() as i64],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .ok()
            .flatten()
            .and_then(|s| s.parse::<ulid::Ulid>().ok().map(TurnId))
    }

    /// If `referenced_message_id` is mapped, inject a `[turn:<id>]` token at the start of `text`.
    /// Returns the (possibly modified) text.
    pub fn inject_turn_ref(&self, text: &str, referenced_message_id: Option<&MessageId>) -> String {
        match referenced_message_id.and_then(|id| self.get(id)) {
            Some(turn_id) => {
                let token = turn_ref::construct(turn_id);
                format!("{token} {text}")
            }
            None => text.to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn_id(bits: u128) -> TurnId {
        TurnId(ulid::Ulid::from(bits))
    }

    #[test]
    fn turn_map_inject_token() {
        let mut map = TurnMap::in_memory().unwrap();
        let msg_id = MessageId::new(123);
        let tid = turn_id(42);
        map.record(msg_id, tid);

        let text = "what did you mean by that?";
        let injected = map.inject_turn_ref(text, Some(&msg_id));
        let token = turn_ref::construct(tid);
        assert_eq!(injected, format!("{token} {text}"));
    }

    #[test]
    fn turn_map_miss_no_inject() {
        let map = TurnMap::in_memory().unwrap();
        let msg_id = MessageId::new(999);
        let text = "a normal message";
        let injected = map.inject_turn_ref(text, Some(&msg_id));
        assert_eq!(injected, text);
    }

    #[test]
    fn turn_map_none_reference_no_inject() {
        let map = TurnMap::in_memory().unwrap();
        let text = "a standalone message";
        let injected = map.inject_turn_ref(text, None);
        assert_eq!(injected, text);
    }

    #[test]
    fn turn_map_persists_across_connections() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("turn_map.db");

        let msg_id = MessageId::new(456);
        let tid = turn_id(99);

        {
            let mut map = TurnMap::open(&path).unwrap();
            map.record(msg_id, tid);
        }

        {
            let map = TurnMap::open(&path).unwrap();
            assert_eq!(map.get(&msg_id), Some(tid));
        }
    }

    #[test]
    fn turn_map_record_replaces() {
        let mut map = TurnMap::in_memory().unwrap();
        let msg_id = MessageId::new(789);
        let tid1 = turn_id(1);
        let tid2 = turn_id(2);
        map.record(msg_id, tid1);
        map.record(msg_id, tid2);
        assert_eq!(map.get(&msg_id), Some(tid2));
    }
}
