//! Context sync: keeps a channel's context memory current — writing channel metadata and laconic
//! guidance on first contact, and superseding the descriptor in place when the channel's name or topic
//! changes.
//!
//! State — the last-written metadata text and the entry id per channel — persists in SQLite (the same
//! file the turn map and projection sync share), so a connector restart recovers the entry id to
//! supersede rather than re-appending a duplicate descriptor on the next message.

use std::{path::PathBuf, time::Duration};

use rusqlite::{Connection, OptionalExtension, params};
use serenity::model::id::ChannelId;
use tokio::sync::Mutex;

use zuihitsu_core::ids::{ConversationLocator, EntryId};
use zuihitsu_platform_connector_api::{ContextEntry, PlatformClient};

use crate::error::Result;

/// The laconic guidance text for a Discord guild channel.
const CHANNEL_GUIDANCE: &str = "This is a Discord channel. Be laconic — one paragraph at most. \
    Use Discord markdown sparingly. Don't acknowledge every message.";

/// The laconic guidance text for a Discord DM.
const DM_GUIDANCE: &str = "This is a Discord DM. Be conversational but still concise.";

/// The persisted per-channel context state — the last-written metadata text and the entry id it landed
/// on, keyed by channel id.
///
/// Created with a path, the state persists to disk so a restart recovers the entry id to supersede.
/// Created in memory, it is lost on restart (tests only).
pub struct ContextSync {
    conn: Mutex<Connection>,
}

impl ContextSync {
    /// Open persistent context state at `path`, creating the database and schema if absent.
    pub fn open(path: &PathBuf) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        Self::init(&conn)?;
        Ok(ContextSync {
            conn: Mutex::new(conn),
        })
    }

    /// Create in-memory context state (lost on restart). Used in tests.
    #[cfg(test)]
    pub fn in_memory() -> rusqlite::Result<Self> {
        let conn = Connection::open_in_memory()?;
        Self::init(&conn)?;
        Ok(ContextSync {
            conn: Mutex::new(conn),
        })
    }

    fn init(conn: &Connection) -> rusqlite::Result<()> {
        // The turn map and projection sync share this file through their own connections, so a writer
        // waits for the other's brief write lock rather than failing `SQLITE_BUSY`.
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS context_sync (
                channel_id TEXT PRIMARY KEY,
                metadata   TEXT NOT NULL,
                entry_id   TEXT
            );",
        )
    }

    /// Sync a channel's freshly-composed context `metadata` to the server: an unchanged descriptor makes
    /// no server call and returns `false`; a changed or first-seen descriptor calls `write_context`,
    /// superseding the entry the prior write returned (`None` on first contact), persists the new entry
    /// id and text, and returns `true`.
    ///
    /// The lock is held across the whole read-write-record cycle, so two events for the same channel
    /// cannot both read the same prior state and double-write. Writes fire only on a change, so
    /// serializing them is cheap.
    pub async fn sync(
        &self,
        client: &PlatformClient,
        locator: &ConversationLocator,
        channel_id: ChannelId,
        metadata: String,
    ) -> Result<bool> {
        let conn = self.conn.lock().await;

        let stored = read_state(&conn, channel_id);
        if stored.as_ref().is_some_and(|(text, _)| text == &metadata) {
            return Ok(false);
        }

        let supersedes = stored.and_then(|(_, entry_id)| entry_id);
        let entries = vec![ContextEntry {
            text: metadata.clone(),
            supersedes,
        }];
        let response = client.write_context(locator, &entries).await?;

        write_state(
            &conn,
            channel_id,
            &metadata,
            response.entries.into_iter().next(),
        );
        Ok(true)
    }
}

/// Compose the context metadata for a Discord guild channel.
pub fn channel_metadata_text(guild_name: &str, channel_name: &str, topic: &str) -> String {
    format!("Channel: {guild_name} / {channel_name}. Topic: {topic}. {CHANNEL_GUIDANCE}")
}

/// Compose the context metadata for a Discord DM.
pub fn dm_metadata_text() -> String {
    DM_GUIDANCE.to_owned()
}

/// Read the last-written `(metadata, entry_id)` for `channel_id`, or `None` if never written. The stored
/// entry id may itself be `None` — a first-contact write whose server response carried no id.
fn read_state(conn: &Connection, channel_id: ChannelId) -> Option<(String, Option<EntryId>)> {
    conn.query_row(
        "SELECT metadata, entry_id FROM context_sync WHERE channel_id = ?1",
        params![channel_id.get().to_string()],
        |row| {
            let metadata: String = row.get("metadata")?;
            let entry_id: Option<String> = row.get("entry_id")?;
            Ok((metadata, entry_id))
        },
    )
    .optional()
    .ok()
    .flatten()
    .map(|(metadata, entry_id)| {
        let entry_id = entry_id.and_then(|s| s.parse::<ulid::Ulid>().ok().map(EntryId));
        (metadata, entry_id)
    })
}

/// Record the new `(metadata, entry_id)` for `channel_id`, replacing any prior row.
fn write_state(
    conn: &Connection,
    channel_id: ChannelId,
    metadata: &str,
    entry_id: Option<EntryId>,
) {
    let _ = conn.execute(
        "INSERT OR REPLACE INTO context_sync (channel_id, metadata, entry_id)
         VALUES (?1, ?2, ?3)",
        params![
            channel_id.get().to_string(),
            metadata,
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

    fn channel(id: u64) -> ChannelId {
        ChannelId::new(id)
    }

    #[tokio::test]
    async fn state_round_trips_including_a_null_entry_id() {
        let sync = ContextSync::in_memory().unwrap();
        let conn = sync.conn.lock().await;

        // An unseen channel has no state, so a sync would treat the metadata as first contact.
        assert_eq!(read_state(&conn, channel(1)), None);

        // A written descriptor round-trips with its entry id — the value the change check compares
        // against and the id the next write supersedes.
        write_state(
            &conn,
            channel(1),
            "Channel: Acme / general.",
            Some(entry_id(7)),
        );
        assert_eq!(
            read_state(&conn, channel(1)),
            Some(("Channel: Acme / general.".to_owned(), Some(entry_id(7))))
        );

        // A rename replaces the row, carrying the new entry id forward.
        write_state(
            &conn,
            channel(1),
            "Channel: Acme / chat.",
            Some(entry_id(8)),
        );
        assert_eq!(
            read_state(&conn, channel(1)),
            Some(("Channel: Acme / chat.".to_owned(), Some(entry_id(8))))
        );

        // A first-contact write whose response carried no id records a null entry id — distinct from
        // never-seen, so the next change still recognises the descriptor as stored.
        write_state(
            &conn,
            channel(2),
            "This is a Discord DM. Be conversational.",
            None,
        );
        assert_eq!(
            read_state(&conn, channel(2)),
            Some(("This is a Discord DM. Be conversational.".to_owned(), None))
        );

        // Channels are independent.
        assert_eq!(
            read_state(&conn, channel(1)),
            Some(("Channel: Acme / chat.".to_owned(), Some(entry_id(8))))
        );
    }

    #[test]
    fn metadata_composition_reflects_the_channel_and_dm_split() {
        // The change check keys on this exact text, so the composition is the sync decision's input: an
        // identical channel/topic composes an identical descriptor (no write), a rename composes a
        // different one (a superseding write), and a DM composes its own laconic guidance.
        let general = channel_metadata_text("Acme", "general", "Team chat");
        assert_eq!(
            channel_metadata_text("Acme", "general", "Team chat"),
            general
        );
        assert_ne!(
            channel_metadata_text("Acme", "random", "Team chat"),
            general
        );
        assert!(general.contains(CHANNEL_GUIDANCE));

        let dm = dm_metadata_text();
        assert_eq!(dm, DM_GUIDANCE);
        assert_ne!(dm, general);
    }
}
