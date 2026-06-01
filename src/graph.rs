//! The materialized graph: a pure projection of the event log into queryable SQLite tables.
//!
//! Derived state — it can be dropped and rebuilt from the log at any time without data loss (spec
//! §Storage). The materializer folds events in `Seq` order via [`Graph::apply`], dispatching on the
//! payload's `(type, version)`. Reads are agent-facing, so soft-deleted memories are filtered out.
//! This module is the data-model projection only; intelligence (search ranking, regeneration,
//! visibility) arrives in later stages. Links and FTS search land in the next increment.

use rusqlite::{Connection, OptionalExtension, params};
use ulid::Ulid;

use crate::event::{Event, EventPayload, Volatility};
use crate::ids::{EntryId, MemoryId, MemoryName, Seq, TagName, Timestamp};
use crate::store::Store;

/// A memory as projected, with its applied tags. Soft-deleted memories are never returned here.
#[derive(Clone, Debug, PartialEq)]
pub struct MemoryView {
    pub id: MemoryId,
    pub name: MemoryName,
    pub description: String,
    pub volatility: Volatility,
    pub created_at: Timestamp,
    pub tags: Vec<TagName>,
}

/// A content entry as projected, ordered within its memory by commit order.
#[derive(Clone, Debug, PartialEq)]
pub struct EntryView {
    pub entry_id: EntryId,
    pub asserted_at: Timestamp,
    pub text: String,
}

/// A failure projecting or querying the graph. Display messages are lowercase fragments suitable
/// for "failed to {…}".
#[derive(Debug)]
pub enum GraphError {
    Backend(String),
}

impl std::fmt::Display for GraphError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GraphError::Backend(message) => write!(f, "access the materialized graph: {message}"),
        }
    }
}

impl std::error::Error for GraphError {}

pub struct Graph {
    conn: Connection,
}

impl Graph {
    /// Open (creating if absent) a file-backed graph.
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Graph, GraphError> {
        Self::init(Connection::open(path).map_err(backend)?)
    }

    /// Open an ephemeral in-memory graph — the no-file-I/O configuration tests use.
    pub fn open_in_memory() -> Result<Graph, GraphError> {
        Self::init(Connection::open_in_memory().map_err(backend)?)
    }

    fn init(conn: Connection) -> Result<Graph, GraphError> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS memories (
                 id          TEXT    PRIMARY KEY,
                 name        TEXT    NOT NULL UNIQUE,
                 description TEXT    NOT NULL DEFAULT '',
                 volatility  TEXT    NOT NULL DEFAULT 'Medium',
                 deleted     INTEGER NOT NULL DEFAULT 0,
                 created_at  INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS content_entries (
                 entry_id    TEXT    PRIMARY KEY,
                 memory_id   TEXT    NOT NULL,
                 asserted_at INTEGER NOT NULL,
                 text        TEXT    NOT NULL,
                 seq         INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_entries_memory ON content_entries(memory_id);
             CREATE TABLE IF NOT EXISTS tags (
                 name        TEXT PRIMARY KEY,
                 description TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS memory_tags (
                 memory_id TEXT NOT NULL,
                 tag       TEXT NOT NULL,
                 PRIMARY KEY (memory_id, tag)
             );
             CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value INTEGER NOT NULL);",
        )
        .map_err(backend)?;
        Ok(Graph { conn })
    }

    /// The highest `Seq` applied so far, or `Seq::ZERO` for a fresh graph. Replay resumes from
    /// `head().next()`, which is how a stale graph catches up to log-head.
    pub fn head(&self) -> Result<Seq, GraphError> {
        let value: Option<i64> = self
            .conn
            .query_row("SELECT value FROM meta WHERE key = 'graph_head'", [], |r| {
                r.get(0)
            })
            .optional()
            .map_err(backend)?;
        Ok(Seq(value.unwrap_or(0) as u64))
    }

    /// Replay every event the store holds beyond the current head, applying each. Returns the count
    /// applied. The same machinery catches up a stale graph and rebuilds a fresh one.
    pub fn materialize_from(&mut self, store: &dyn Store) -> Result<usize, GraphError> {
        let from = self.head()?.next();
        let events = store
            .read_from(from)
            .map_err(|e| GraphError::Backend(e.to_string()))?;
        for event in &events {
            self.apply(event)?;
        }
        Ok(events.len())
    }

    /// Fold a single event into the projection, then advance the head. The match arm is the
    /// `(type, version)` dispatch; a wrong arm is a silent-leak class the eval harness backstops.
    pub fn apply(&mut self, event: &Event) -> Result<(), GraphError> {
        match &event.payload {
            EventPayload::GenesisCompleted { .. } => {}
            EventPayload::MemoryCreated { id, name } => {
                self.conn
                    .execute(
                        "INSERT INTO memories (id, name, created_at) VALUES (?1, ?2, ?3)",
                        params![
                            id.0.to_string(),
                            name.as_str(),
                            event.recorded_at.as_millis()
                        ],
                    )
                    .map_err(backend)?;
            }
            EventPayload::MemoryRenamed { id, new_name, .. } => {
                self.conn
                    .execute(
                        "UPDATE memories SET name = ?1 WHERE id = ?2",
                        params![new_name.as_str(), id.0.to_string()],
                    )
                    .map_err(backend)?;
            }
            EventPayload::MemoryDeleted { id } => {
                self.conn
                    .execute(
                        "UPDATE memories SET deleted = 1 WHERE id = ?1",
                        params![id.0.to_string()],
                    )
                    .map_err(backend)?;
            }
            EventPayload::MemoryContentAppended {
                id,
                entry_id,
                asserted_at,
                text,
            } => {
                self.conn
                    .execute(
                        "INSERT INTO content_entries (entry_id, memory_id, asserted_at, text, seq)
                         VALUES (?1, ?2, ?3, ?4, ?5)",
                        params![
                            entry_id.0.to_string(),
                            id.0.to_string(),
                            asserted_at.as_millis(),
                            text,
                            event.seq.0 as i64,
                        ],
                    )
                    .map_err(backend)?;
            }
            EventPayload::MemoryDescriptionRegenerated { id, new_text } => {
                self.conn
                    .execute(
                        "UPDATE memories SET description = ?1 WHERE id = ?2",
                        params![new_text, id.0.to_string()],
                    )
                    .map_err(backend)?;
            }
            EventPayload::MemoryVolatilitySet { id, volatility } => {
                self.conn
                    .execute(
                        "UPDATE memories SET volatility = ?1 WHERE id = ?2",
                        params![volatility.as_str(), id.0.to_string()],
                    )
                    .map_err(backend)?;
            }
            EventPayload::TagCreated { name, description } => {
                self.conn
                    .execute(
                        "INSERT INTO tags (name, description) VALUES (?1, ?2)",
                        params![name.as_str(), description],
                    )
                    .map_err(backend)?;
            }
            EventPayload::TagDescriptionChanged {
                name,
                new_description,
            } => {
                self.conn
                    .execute(
                        "UPDATE tags SET description = ?1 WHERE name = ?2",
                        params![new_description, name.as_str()],
                    )
                    .map_err(backend)?;
            }
            EventPayload::TagAppliedToMemory { memory, tag } => {
                self.conn
                    .execute(
                        "INSERT OR IGNORE INTO memory_tags (memory_id, tag) VALUES (?1, ?2)",
                        params![memory.0.to_string(), tag.as_str()],
                    )
                    .map_err(backend)?;
            }
            EventPayload::TagRemovedFromMemory { memory, tag } => {
                self.conn
                    .execute(
                        "DELETE FROM memory_tags WHERE memory_id = ?1 AND tag = ?2",
                        params![memory.0.to_string(), tag.as_str()],
                    )
                    .map_err(backend)?;
            }
        }

        self.conn
            .execute(
                "INSERT INTO meta (key, value) VALUES ('graph_head', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![event.seq.0 as i64],
            )
            .map_err(backend)?;
        Ok(())
    }

    /// Fetch a live (non-deleted) memory by its agent-facing name.
    pub fn memory_by_name(&self, name: &str) -> Result<Option<MemoryView>, GraphError> {
        self.fetch_memory("name", name)
    }

    /// Fetch a live (non-deleted) memory by its internal id.
    pub fn memory_by_id(&self, id: MemoryId) -> Result<Option<MemoryView>, GraphError> {
        self.fetch_memory("id", &id.0.to_string())
    }

    /// All live memories whose name begins with `prefix` (e.g. `"person/"`), ordered by name.
    pub fn memories_in_namespace(&self, prefix: &str) -> Result<Vec<MemoryView>, GraphError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, name, description, volatility, created_at FROM memories
                 WHERE name LIKE ?1 || '%' AND deleted = 0 ORDER BY name",
            )
            .map_err(backend)?;
        let rows = stmt
            .query_map(params![prefix], row_to_memory_columns)
            .map_err(backend)?;

        let mut memories = Vec::new();
        for row in rows {
            memories.push(self.assemble_memory(row.map_err(backend)?)?);
        }
        Ok(memories)
    }

    /// A memory's content entries, in commit order. Low-level: not filtered by soft delete.
    pub fn entries(&self, id: MemoryId) -> Result<Vec<EntryView>, GraphError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT entry_id, asserted_at, text FROM content_entries
                 WHERE memory_id = ?1 ORDER BY seq",
            )
            .map_err(backend)?;
        let rows = stmt
            .query_map(params![id.0.to_string()], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .map_err(backend)?;

        let mut entries = Vec::new();
        for row in rows {
            let (entry_id, asserted_at, text) = row.map_err(backend)?;
            entries.push(EntryView {
                entry_id: EntryId(parse_ulid(&entry_id)?),
                asserted_at: Timestamp::from_millis(asserted_at),
                text,
            });
        }
        Ok(entries)
    }

    /// A tag's description, or `None` if the tag was never created.
    pub fn tag_description(&self, name: &str) -> Result<Option<String>, GraphError> {
        self.conn
            .query_row(
                "SELECT description FROM tags WHERE name = ?1",
                params![name],
                |r| r.get(0),
            )
            .optional()
            .map_err(backend)
    }

    fn fetch_memory(&self, column: &str, value: &str) -> Result<Option<MemoryView>, GraphError> {
        let sql = format!(
            "SELECT id, name, description, volatility, created_at FROM memories
             WHERE {column} = ?1 AND deleted = 0"
        );
        let row = self
            .conn
            .query_row(&sql, params![value], row_to_memory_columns)
            .optional()
            .map_err(backend)?;
        match row {
            Some(columns) => Ok(Some(self.assemble_memory(columns)?)),
            None => Ok(None),
        }
    }

    fn assemble_memory(&self, columns: MemoryColumns) -> Result<MemoryView, GraphError> {
        let (id, name, description, volatility, created_at) = columns;
        Ok(MemoryView {
            id: MemoryId(parse_ulid(&id)?),
            name: MemoryName::new(name),
            description,
            volatility: Volatility::parse(&volatility)
                .ok_or_else(|| GraphError::Backend(format!("unknown volatility {volatility:?}")))?,
            created_at: Timestamp::from_millis(created_at),
            tags: self.tags_of(&id)?,
        })
    }

    fn tags_of(&self, memory_id: &str) -> Result<Vec<TagName>, GraphError> {
        let mut stmt = self
            .conn
            .prepare("SELECT tag FROM memory_tags WHERE memory_id = ?1 ORDER BY tag")
            .map_err(backend)?;
        let rows = stmt
            .query_map(params![memory_id], |r| r.get::<_, String>(0))
            .map_err(backend)?;
        let mut tags = Vec::new();
        for row in rows {
            tags.push(TagName::new(row.map_err(backend)?));
        }
        Ok(tags)
    }
}

/// The raw memory columns, shared by the single- and multi-row read paths.
type MemoryColumns = (String, String, String, String, i64);

fn row_to_memory_columns(row: &rusqlite::Row<'_>) -> rusqlite::Result<MemoryColumns> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
    ))
}

fn parse_ulid(text: &str) -> Result<Ulid, GraphError> {
    Ulid::from_string(text).map_err(|e| GraphError::Backend(format!("invalid ulid {text:?}: {e}")))
}

fn backend(error: rusqlite::Error) -> GraphError {
    GraphError::Backend(error.to_string())
}
