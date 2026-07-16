//! Participant identity sync: projects each Discord user's current username, display name, and server
//! nickname onto their zuihitsu profile, superseding the prior value when it changes and retracting it
//! when it is cleared. The connector holds the entry id each projection returned, so the server
//! supersedes or retracts by id without keying attributes itself.
//!
//! State — the last-seen raw value and the entry id per `(user, attribute)` — persists in SQLite, so a
//! connector restart keeps superseding in place rather than re-appending a duplicate. The attribute key
//! carries the guild id for a nickname, since a user may be nicknamed differently in each server the bot
//! shares with them, while the username and display name are global to the account.

use std::path::PathBuf;

use rusqlite::{Connection, OptionalExtension, params};
use tokio::sync::Mutex;

use zuihitsu_connector_api::{ParticipantAttribute, PlatformClient};
use zuihitsu_core::ids::{EntryId, PersonId};

use crate::error::Result;

/// One identity attribute observed for a user on a message: a stable `key` (so a per-guild nickname
/// stays distinct from the global username), the raw `value` for change detection (`None` when the
/// attribute is not set), and the `entry_text` to record when it is set.
pub struct ObservedAttribute {
    pub key: String,
    pub value: Option<String>,
    pub entry_text: String,
}

/// The persisted last-projected identity state, keyed by `(user, attribute)`.
///
/// Created with a path, the state persists to disk so a restart recovers the entry ids to supersede.
/// Created in memory, it is lost on restart (tests only).
pub struct ParticipantSync {
    conn: Mutex<Connection>,
}

impl ParticipantSync {
    /// Open persistent identity state at `path`, creating the database and schema if absent.
    pub fn open(path: &PathBuf) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        Self::init(&conn)?;
        Ok(ParticipantSync {
            conn: Mutex::new(conn),
        })
    }

    /// Create in-memory identity state (lost on restart). Used in tests.
    #[cfg(test)]
    pub fn in_memory() -> rusqlite::Result<Self> {
        let conn = Connection::open_in_memory()?;
        Self::init(&conn)?;
        Ok(ParticipantSync {
            conn: Mutex::new(conn),
        })
    }

    fn init(conn: &Connection) -> rusqlite::Result<()> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS participant_sync (
                user_id  TEXT NOT NULL,
                attr_key TEXT NOT NULL,
                value    TEXT,
                entry_id TEXT,
                PRIMARY KEY (user_id, attr_key)
            );",
        )
    }

    /// Project `person`'s observed attributes, sending only those whose value changed since the last
    /// projection: a changed value supersedes the entry the prior projection returned, a now-`None`
    /// value retracts it. Records the new value and entry id per changed attribute. A message that
    /// changes nothing makes no server call.
    ///
    /// The lock is held across the whole read-project-record cycle, so two messages from the same user
    /// cannot both read the same prior state and double-project. Syncs fire only on a change, so
    /// serializing them is cheap.
    pub async fn sync(
        &self,
        client: &PlatformClient,
        person: &PersonId,
        observed: &[ObservedAttribute],
    ) -> Result<()> {
        let conn = self.conn.lock().await;
        let user_id = person.id.as_str();

        let mut changed: Vec<(&str, Option<String>)> = Vec::new();
        let mut attributes: Vec<ParticipantAttribute> = Vec::new();
        for obs in observed {
            let stored = read_state(&conn, user_id, &obs.key);
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
            return Ok(());
        }

        let results = client.project_participant(person, &attributes).await?;

        for ((key, value), entry_id) in changed.into_iter().zip(results) {
            write_state(&conn, user_id, key, value.as_deref(), entry_id);
        }
        Ok(())
    }
}

/// Read the last-projected `(value, entry_id)` for `(user_id, attr_key)`, or `None` if never projected.
/// A stored value or entry id may itself be `None` — a cleared attribute records a null value.
fn read_state(
    conn: &Connection,
    user_id: &str,
    attr_key: &str,
) -> Option<(Option<String>, Option<EntryId>)> {
    conn.query_row(
        "SELECT value, entry_id FROM participant_sync WHERE user_id = ?1 AND attr_key = ?2",
        params![user_id, attr_key],
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

/// Record the new `(value, entry_id)` for `(user_id, attr_key)`, replacing any prior row.
fn write_state(
    conn: &Connection,
    user_id: &str,
    attr_key: &str,
    value: Option<&str>,
    entry_id: Option<EntryId>,
) {
    let _ = conn.execute(
        "INSERT OR REPLACE INTO participant_sync (user_id, attr_key, value, entry_id)
         VALUES (?1, ?2, ?3, ?4)",
        params![
            user_id,
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
        let sync = ParticipantSync::in_memory().unwrap();
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

        // Keys are independent, so a per-guild nickname is tracked apart from the global username.
        write_state(&conn, "42", "nickname:7", Some("Cap"), Some(entry_id(3)));
        assert_eq!(
            read_state(&conn, "42", "nickname:7"),
            Some((Some("Cap".to_owned()), Some(entry_id(3))))
        );
        assert_eq!(read_state(&conn, "42", "username"), Some((None, None)));
    }
}
