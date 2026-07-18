//! Projection sync: projects a subject's current attributes onto its zuihitsu memory, superseding the
//! prior value when it changes and retracting it when it is cleared. A subject is a participant (their
//! username, display name, and per-guild nickname) or a guild (its server name); both project the same
//! way, onto their `person/*` or `context/*` memory respectively. The connector holds the entry id each
//! projection returned, so the server supersedes or retracts by id without keying attributes itself.
//!
//! State — the last-seen raw value and the entry id per `(subject, attribute)` — persists in SQLite, so
//! a connector restart keeps superseding in place rather than re-appending a duplicate. The subject key
//! namespaces the two kinds (a participant by user id, a guild by its scope path), and the attribute key
//! carries the guild id for a nickname, since a user may be nicknamed differently in each server.

use std::{path::PathBuf, time::Duration};

use rusqlite::{Connection, OptionalExtension, params};
use tokio::sync::Mutex;

use zuihitsu_core::ids::EntryId;
use zuihitsu_platform_connector_api::{LinkEndpoint, ParticipantAttribute, PlatformClient};

use crate::error::Result;

/// One attribute observed for a subject on an event: a stable `key` (so a per-guild nickname stays
/// distinct from the global username), the raw `value` for change detection (`None` when the attribute
/// is not set), and the `entry_text` to record when it is set.
pub struct ObservedAttribute {
    pub key: String,
    pub value: Option<String>,
    pub entry_text: String,
}

/// The persisted last-projected attribute state, keyed by `(subject, attribute)`.
///
/// Created with a path, the state persists to disk so a restart recovers the entry ids to supersede.
/// Created in memory, it is lost on restart (tests only).
pub struct ProjectionSync {
    conn: Mutex<Connection>,
}

impl ProjectionSync {
    /// Open persistent projection state at `path`, creating the database and schema if absent.
    pub fn open(path: &PathBuf) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        Self::init(&conn)?;
        Ok(ProjectionSync {
            conn: Mutex::new(conn),
        })
    }

    /// Create in-memory projection state (lost on restart). Used in tests.
    #[cfg(test)]
    pub fn in_memory() -> rusqlite::Result<Self> {
        let conn = Connection::open_in_memory()?;
        Self::init(&conn)?;
        Ok(ProjectionSync {
            conn: Mutex::new(conn),
        })
    }

    fn init(conn: &Connection) -> rusqlite::Result<()> {
        // The turn map shares this file through its own connection, so a writer waits for the other's
        // brief write lock rather than failing `SQLITE_BUSY`.
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS projection_sync (
                subject  TEXT NOT NULL,
                attr_key TEXT NOT NULL,
                value    TEXT,
                entry_id TEXT,
                PRIMARY KEY (subject, attr_key)
            );",
        )
    }

    /// Project a subject's observed attributes onto `target`, sending only those whose value changed
    /// since the last projection: a changed value supersedes the entry the prior projection returned, a
    /// now-`None` value retracts it. Records the new value and entry id per changed attribute. An event
    /// that changes nothing makes no server call. `subject` is the persisted state key — a participant's
    /// user id, or a guild's scope path — kept apart so the two kinds never collide.
    ///
    /// The lock is held across the whole read-project-record cycle, so two events for the same subject
    /// cannot both read the same prior state and double-project. Syncs fire only on a change, so
    /// serializing them is cheap.
    pub async fn sync(
        &self,
        client: &PlatformClient,
        target: &LinkEndpoint,
        subject: &str,
        observed: &[ObservedAttribute],
    ) -> Result<()> {
        let conn = self.conn.lock().await;

        let mut changed: Vec<(&str, Option<String>)> = Vec::new();
        let mut attributes: Vec<ParticipantAttribute> = Vec::new();
        for obs in observed {
            let stored = read_state(&conn, subject, &obs.key);
            let last_value = stored.as_ref().and_then(|(value, _)| value.clone());
            if last_value == obs.value {
                continue;
            }
            let supersedes = stored.and_then(|(_, entry_id)| entry_id);
            let text = obs.value.as_ref().map(|_| obs.entry_text.clone());
            attributes.push(ParticipantAttribute { text, supersedes });
            changed.push((obs.key.as_str(), obs.value.clone()));
        }
        if attributes.is_empty() {
            tracing::debug!(
                subject,
                "attributes unchanged since last projection — nothing to send"
            );
            return Ok(());
        }

        let keys: Vec<&str> = changed.iter().map(|(key, _)| *key).collect();
        tracing::info!(subject, ?keys, "projecting attributes");
        let results = client.project(target, &attributes).await?;

        for ((key, value), entry_id) in changed.into_iter().zip(results) {
            write_state(&conn, subject, key, value.as_deref(), entry_id);
        }
        Ok(())
    }
}

/// Read the last-projected `(value, entry_id)` for `(subject, attr_key)`, or `None` if never projected.
/// A stored value or entry id may itself be `None` — a cleared attribute records a null value.
fn read_state(
    conn: &Connection,
    subject: &str,
    attr_key: &str,
) -> Option<(Option<String>, Option<EntryId>)> {
    conn.query_row(
        "SELECT value, entry_id FROM projection_sync WHERE subject = ?1 AND attr_key = ?2",
        params![subject, attr_key],
        |row| {
            let value: Option<String> = row.get("value")?;
            let entry_id: Option<String> = row.get("entry_id")?;
            Ok((value, entry_id))
        },
    )
    .optional()
    .ok()
    .flatten()
    .map(|(value, entry_id)| {
        let entry_id = entry_id.and_then(|s| s.parse::<ulid::Ulid>().ok().map(EntryId));
        (value, entry_id)
    })
}

/// Record the new `(value, entry_id)` for `(subject, attr_key)`, replacing any prior row.
fn write_state(
    conn: &Connection,
    subject: &str,
    attr_key: &str,
    value: Option<&str>,
    entry_id: Option<EntryId>,
) {
    let _ = conn.execute(
        "INSERT OR REPLACE INTO projection_sync (subject, attr_key, value, entry_id)
         VALUES (?1, ?2, ?3, ?4)",
        params![
            subject,
            attr_key,
            value,
            entry_id.map(|id| id.0.to_string())
        ],
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry_id(bits: u128) -> EntryId {
        EntryId(ulid::Ulid::from(bits))
    }

    #[tokio::test]
    async fn state_round_trips_including_nulls() {
        let sync = ProjectionSync::in_memory().unwrap();
        let conn = sync.conn.lock().await;

        // An unseen attribute has no state.
        assert_eq!(read_state(&conn, "42", "username"), None);

        // A recorded value round-trips with its entry id.
        write_state(&conn, "42", "username", Some("dave"), Some(entry_id(1)));
        assert_eq!(
            read_state(&conn, "42", "username"),
            Some((Some("dave".to_owned()), Some(entry_id(1))))
        );

        // A change replaces the row.
        write_state(&conn, "42", "username", Some("davey"), Some(entry_id(2)));
        assert_eq!(
            read_state(&conn, "42", "username"),
            Some((Some("davey".to_owned()), Some(entry_id(2))))
        );

        // A cleared attribute records a null value and entry id — distinct from never-seen.
        write_state(&conn, "42", "username", None, None);
        assert_eq!(read_state(&conn, "42", "username"), Some((None, None)));

        // Subjects are independent, so a guild's server name is tracked apart from a participant's.
        write_state(
            &conn,
            "guild/7",
            "server_name",
            Some("Acme"),
            Some(entry_id(3)),
        );
        assert_eq!(
            read_state(&conn, "guild/7", "server_name"),
            Some((Some("Acme".to_owned()), Some(entry_id(3))))
        );
        assert_eq!(read_state(&conn, "42", "username"), Some((None, None)));
    }
}
