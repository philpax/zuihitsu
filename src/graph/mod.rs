//! The materialized graph: a pure projection of the event log into queryable SQLite tables.
//!
//! Derived state — it can be dropped and rebuilt from the log at any time without data loss (spec
//! §Storage). This root holds the schema, the open/boot path, and the shared types and helpers; the
//! [`Graph::apply`] materializer lives in [`apply`], and the agent-facing query methods in [`read`].

use rusqlite::{Connection, OptionalExtension};
use ulid::Ulid;

use crate::{
    event::{Cardinality, Teller, Visibility, Volatility},
    ids::{ConversationId, EntryId, MemoryId, MemoryName, Seq, SessionId, TurnId},
    store::{Store, StoreError},
    time::Timestamp,
    vocabulary::{RelationName, TagName},
};

mod apply;
mod read;

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

/// A content entry as projected, ordered within its memory by commit order. `occurred_sort` is the
/// denormalized representative instant of the entry's `occurred_at` (spec §Time), or `None` when the
/// entry carries no occurrence (or only a `Recurring` one); recency ranking reads it.
#[derive(Clone, Debug, PartialEq)]
pub struct EntryView {
    pub entry_id: EntryId,
    pub asserted_at: Timestamp,
    pub occurred_sort: Option<Timestamp>,
    pub text: String,
    pub told_by: Teller,
    pub told_in: Option<MemoryId>,
    pub visibility: Visibility,
}

impl EntryView {
    /// Assemble from projected columns, deserializing the structured `told_by` / `told_in` /
    /// `visibility` metadata.
    fn from_db(
        entry_id: EntryId,
        asserted_at: i64,
        occurred_sort: Option<i64>,
        text: String,
        told_by: &str,
        told_in: Option<&str>,
        visibility: &str,
    ) -> Result<EntryView, GraphError> {
        Ok(EntryView {
            entry_id,
            asserted_at: Timestamp::from_millis(asserted_at),
            occurred_sort: occurred_sort.map(Timestamp::from_millis),
            text,
            told_by: serde_json::from_str(told_by)?,
            told_in: told_in.map(|id| parse_ulid(id).map(MemoryId)).transpose()?,
            visibility: serde_json::from_str(visibility)?,
        })
    }
}

/// A registered relation as projected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RelationView {
    pub name: RelationName,
    pub inverse: RelationName,
    pub from_card: Cardinality,
    pub to_card: Cardinality,
    pub symmetric: bool,
    pub reflexive: bool,
}

/// A stored edge in its canonical direction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkView {
    pub from: MemoryId,
    pub to: MemoryId,
    pub relation: RelationName,
}

/// A session as projected: its conversation, when it opened, the carryover extent (if it opened via
/// compaction), the captured brief, and its participants (the present set at open, plus anyone who
/// joined mid-session).
#[derive(Clone, Debug, PartialEq)]
pub struct SessionView {
    pub id: SessionId,
    pub conversation: ConversationId,
    pub started_at: Timestamp,
    pub seeded_from_turn: Option<TurnId>,
    pub brief: String,
    pub participants: Vec<MemoryId>,
}

/// A failure projecting or querying the graph.
#[derive(Debug)]
pub enum GraphError {
    /// The SQLite backend failed.
    Backend(rusqlite::Error),
    /// Reading the log to project from it failed.
    Store(StoreError),
    /// An entry's structured metadata (`told_by` / `visibility`) could not be (de)serialized.
    Serialize(serde_json::Error),
    /// A projected value could not be interpreted — a malformed id or an unknown enum tag — which
    /// means the projection is corrupt (a materializer bug or external tampering), not a typed
    /// failure with a source to delegate to.
    Malformed(String),
}

impl std::fmt::Display for GraphError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GraphError::Backend(error) => write!(f, "materialized graph (backend): {error}"),
            GraphError::Store(error) => write!(f, "materialized graph (store): {error}"),
            GraphError::Serialize(error) => write!(f, "materialized graph (serde): {error}"),
            GraphError::Malformed(message) => {
                write!(f, "materialized graph (malformed): {message}")
            }
        }
    }
}

impl std::error::Error for GraphError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            GraphError::Backend(error) => Some(error),
            GraphError::Store(error) => Some(error),
            GraphError::Serialize(error) => Some(error),
            GraphError::Malformed(_) => None,
        }
    }
}

impl From<rusqlite::Error> for GraphError {
    fn from(error: rusqlite::Error) -> GraphError {
        GraphError::Backend(error)
    }
}

impl From<StoreError> for GraphError {
    fn from(error: StoreError) -> GraphError {
        GraphError::Store(error)
    }
}

impl From<serde_json::Error> for GraphError {
    fn from(error: serde_json::Error) -> GraphError {
        GraphError::Serialize(error)
    }
}

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
                 created_at  INTEGER NOT NULL,
                 class_id    TEXT    NOT NULL DEFAULT ''
             );
             CREATE INDEX IF NOT EXISTS idx_memories_class ON memories(class_id);
             CREATE TABLE IF NOT EXISTS content_entries (
                 entry_id      TEXT    PRIMARY KEY,
                 memory_id     TEXT    NOT NULL,
                 asserted_at   INTEGER NOT NULL,
                 occurred_at   TEXT,
                 occurred_sort INTEGER,
                 occurred_lo   INTEGER,
                 occurred_hi   INTEGER,
                 text          TEXT    NOT NULL,
                 told_by       TEXT    NOT NULL,
                 told_in       TEXT,
                 visibility    TEXT    NOT NULL,
                 seq           INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_entries_memory ON content_entries(memory_id);
             CREATE INDEX IF NOT EXISTS idx_entries_occurred_sort
                 ON content_entries(occurred_sort);
             CREATE INDEX IF NOT EXISTS idx_entries_occurred_lo_hi
                 ON content_entries(occurred_lo, occurred_hi);
             CREATE TABLE IF NOT EXISTS tags (
                 name        TEXT PRIMARY KEY,
                 description TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS memory_tags (
                 memory_id TEXT NOT NULL,
                 tag       TEXT NOT NULL,
                 PRIMARY KEY (memory_id, tag)
             );
             CREATE TABLE IF NOT EXISTS relations (
                 name      TEXT    PRIMARY KEY,
                 inverse   TEXT    NOT NULL,
                 from_card TEXT    NOT NULL,
                 to_card   TEXT    NOT NULL,
                 symmetric INTEGER NOT NULL,
                 reflexive INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS links (
                 from_id  TEXT NOT NULL,
                 to_id    TEXT NOT NULL,
                 relation TEXT NOT NULL,
                 source   TEXT NOT NULL,
                 PRIMARY KEY (from_id, to_id, relation)
             );
             CREATE INDEX IF NOT EXISTS idx_links_to ON links(to_id, relation);
             CREATE TABLE IF NOT EXISTS conversations (
                 id             TEXT    PRIMARY KEY,
                 platform       TEXT    NOT NULL,
                 scope_path     TEXT    NOT NULL,
                 context_memory TEXT    NOT NULL,
                 ended          INTEGER NOT NULL DEFAULT 0
             );
             CREATE UNIQUE INDEX IF NOT EXISTS idx_conversations_locator
                 ON conversations(platform, scope_path);
             CREATE TABLE IF NOT EXISTS sessions (
                 id               TEXT    PRIMARY KEY,
                 conversation     TEXT    NOT NULL,
                 started_at       INTEGER NOT NULL,
                 seeded_from_turn TEXT,
                 brief            TEXT    NOT NULL,
                 ended            INTEGER NOT NULL DEFAULT 0,
                 seq              INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_sessions_conversation ON sessions(conversation);
             CREATE TABLE IF NOT EXISTS session_participants (
                 session TEXT NOT NULL,
                 memory  TEXT NOT NULL,
                 at_turn TEXT,
                 PRIMARY KEY (session, memory)
             );
             CREATE TABLE IF NOT EXISTS participant_identities (
                 platform         TEXT NOT NULL,
                 platform_user_id TEXT NOT NULL,
                 memory           TEXT NOT NULL,
                 PRIMARY KEY (platform, platform_user_id)
             );
             CREATE INDEX IF NOT EXISTS idx_participant_identities_memory
                 ON participant_identities(memory);
             CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value INTEGER NOT NULL);
             CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(
                 name, description, content, memory_id UNINDEXED
             );",
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
        let events = store.read_from(from).map_err(GraphError::Store)?;
        for event in &events {
            self.apply(event)?;
        }
        tracing::debug!(applied = events.len(), from = from.0, "materialized graph");
        Ok(events.len())
    }
}

fn parse_ulid(text: &str) -> Result<Ulid, GraphError> {
    Ulid::from_string(text)
        .map_err(|e| GraphError::Malformed(format!("invalid ulid {text:?}: {e}")))
}

fn backend(error: rusqlite::Error) -> GraphError {
    GraphError::Backend(error)
}
