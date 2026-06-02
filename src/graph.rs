//! The materialized graph: a pure projection of the event log into queryable SQLite tables.
//!
//! Derived state — it can be dropped and rebuilt from the log at any time without data loss (spec
//! §Storage). The materializer folds events in `Seq` order via [`Graph::apply`], dispatching on the
//! payload's `(type, version)`. Reads are agent-facing, so soft-deleted memories are filtered out.
//! This module is the data-model projection only; intelligence (search ranking, regeneration,
//! visibility) arrives in later stages. Links and FTS search land in the next increment.

use rusqlite::{Connection, OptionalExtension, params};
use ulid::Ulid;

use crate::{
    event::{Cardinality, Event, EventPayload, Teller, Visibility, Volatility},
    ids::{EntryId, MemoryId, MemoryName, RelationName, Seq, TagName, Timestamp},
    store::{Store, StoreError},
};

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
        text: String,
        told_by: &str,
        told_in: Option<&str>,
        visibility: &str,
    ) -> Result<EntryView, GraphError> {
        Ok(EntryView {
            entry_id,
            asserted_at: Timestamp::from_millis(asserted_at),
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
                 created_at  INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS content_entries (
                 entry_id    TEXT    PRIMARY KEY,
                 memory_id   TEXT    NOT NULL,
                 asserted_at INTEGER NOT NULL,
                 text        TEXT    NOT NULL,
                 told_by     TEXT    NOT NULL,
                 told_in     TEXT,
                 visibility  TEXT    NOT NULL,
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

    /// Fold a single event into the projection, then advance the head. The match arm is the
    /// `(type, version)` dispatch; a wrong arm is a silent-leak class the eval harness backstops.
    pub fn apply(&mut self, event: &Event) -> Result<(), GraphError> {
        match &event.payload {
            // No graph projection: genesis marker, and orchestration/behavioral config which the
            // server reads from the log rather than the graph.
            EventPayload::GenesisCompleted { .. }
            | EventPayload::PromptTemplateRegistered { .. }
            | EventPayload::ConfigSet { .. }
            | EventPayload::LuaExecuted { .. }
            | EventPayload::ConversationTurn { .. } => {}
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
                self.conn
                    .execute(
                        "INSERT INTO memories_fts (memory_id, name, description, content)
                         VALUES (?1, ?2, '', '')",
                        params![id.0.to_string(), name.as_str()],
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
                self.conn
                    .execute(
                        "UPDATE memories_fts SET name = ?1 WHERE memory_id = ?2",
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
                told_by,
                told_in,
                visibility,
            } => {
                self.conn
                    .execute(
                        "INSERT INTO content_entries \
                         (entry_id, memory_id, asserted_at, text, told_by, told_in, visibility, seq)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                        params![
                            entry_id.0.to_string(),
                            id.0.to_string(),
                            asserted_at.as_millis(),
                            text,
                            serde_json::to_string(told_by).map_err(GraphError::Serialize)?,
                            told_in.map(|memory| memory.0.to_string()),
                            serde_json::to_string(visibility).map_err(GraphError::Serialize)?,
                            event.seq.0 as i64,
                        ],
                    )
                    .map_err(backend)?;
                // Only public content enters the lexical index: name and description are already
                // public-safe, so keeping FTS public-only means a lexical hit needs no visibility
                // filter. Private content stays retrievable only via its (predicate-filtered) entry
                // vector.
                if *visibility == Visibility::Public {
                    self.conn
                        .execute(
                            "UPDATE memories_fts SET content = content || ' ' || ?1
                             WHERE memory_id = ?2",
                            params![text, id.0.to_string()],
                        )
                        .map_err(backend)?;
                }
            }
            EventPayload::MemoryDescriptionRegenerated { id, new_text, .. } => {
                self.conn
                    .execute(
                        "UPDATE memories SET description = ?1 WHERE id = ?2",
                        params![new_text, id.0.to_string()],
                    )
                    .map_err(backend)?;
                self.conn
                    .execute(
                        "UPDATE memories_fts SET description = ?1 WHERE memory_id = ?2",
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
            EventPayload::LinkTypeRegistered {
                name,
                inverse,
                from_card,
                to_card,
                symmetric,
                reflexive,
            } => {
                self.conn
                    .execute(
                        "INSERT INTO relations (name, inverse, from_card, to_card, symmetric, reflexive)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                         ON CONFLICT(name) DO UPDATE SET
                             inverse = excluded.inverse, from_card = excluded.from_card,
                             to_card = excluded.to_card, symmetric = excluded.symmetric,
                             reflexive = excluded.reflexive",
                        params![
                            name.as_str(),
                            inverse.as_str(),
                            from_card.as_str(),
                            to_card.as_str(),
                            i64::from(*symmetric),
                            i64::from(*reflexive),
                        ],
                    )
                    .map_err(backend)?;
            }
            EventPayload::LinkCreated {
                from,
                to,
                relation,
                source,
            } => {
                let edge = self.canonical_edge(*from, *to, relation)?;
                self.conn
                    .execute(
                        "INSERT OR IGNORE INTO links (from_id, to_id, relation, source)
                         VALUES (?1, ?2, ?3, ?4)",
                        params![edge.0, edge.1, edge.2, source.as_str()],
                    )
                    .map_err(backend)?;
            }
            EventPayload::LinkRemoved { from, to, relation } => {
                let edge = self.canonical_edge(*from, *to, relation)?;
                self.conn
                    .execute(
                        "DELETE FROM links WHERE from_id = ?1 AND to_id = ?2 AND relation = ?3",
                        params![edge.0, edge.1, edge.2],
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
                "SELECT entry_id, asserted_at, text, told_by, told_in, visibility
                 FROM content_entries WHERE memory_id = ?1 ORDER BY seq",
            )
            .map_err(backend)?;
        let rows = stmt
            .query_map(params![id.0.to_string()], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, String>(5)?,
                ))
            })
            .map_err(backend)?;

        let mut entries = Vec::new();
        for row in rows {
            let (entry_id, asserted_at, text, told_by, told_in, visibility) =
                row.map_err(backend)?;
            entries.push(EntryView::from_db(
                EntryId(parse_ulid(&entry_id)?),
                asserted_at,
                text,
                &told_by,
                told_in.as_deref(),
                &visibility,
            )?);
        }
        Ok(entries)
    }

    /// A single entry by id, with its live owning memory — or `None` if the entry is unknown or its
    /// memory is soft-deleted. The visibility predicate needs both: the entry's teller/visibility and
    /// the memory's subject. Used to resolve and filter an entry-vector search hit.
    pub fn entry_by_id(
        &self,
        entry_id: EntryId,
    ) -> Result<Option<(MemoryView, EntryView)>, GraphError> {
        let row = self
            .conn
            .query_row(
                "SELECT memory_id, asserted_at, text, told_by, told_in, visibility
                 FROM content_entries WHERE entry_id = ?1",
                params![entry_id.0.to_string()],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, Option<String>>(4)?,
                        r.get::<_, String>(5)?,
                    ))
                },
            )
            .optional()
            .map_err(backend)?;
        let Some((memory_id, asserted_at, text, told_by, told_in, visibility)) = row else {
            return Ok(None);
        };
        let Some(memory) = self.memory_by_id(MemoryId(parse_ulid(&memory_id)?))? else {
            return Ok(None);
        };
        let entry = EntryView::from_db(
            entry_id,
            asserted_at,
            text,
            &told_by,
            told_in.as_deref(),
            &visibility,
        )?;
        Ok(Some((memory, entry)))
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

    /// A registered relation by its canonical name, or `None`.
    pub fn relation(&self, name: &str) -> Result<Option<RelationView>, GraphError> {
        self.conn
            .query_row(
                "SELECT name, inverse, from_card, to_card, symmetric, reflexive
                 FROM relations WHERE name = ?1",
                params![name],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, i64>(4)?,
                        r.get::<_, i64>(5)?,
                    ))
                },
            )
            .optional()
            .map_err(backend)?
            .map(
                |(name, inverse, from_card, to_card, symmetric, reflexive)| {
                    Ok(RelationView {
                        name: RelationName::new(name),
                        inverse: RelationName::new(inverse),
                        from_card: parse_cardinality(&from_card)?,
                        to_card: parse_cardinality(&to_card)?,
                        symmetric: symmetric != 0,
                        reflexive: reflexive != 0,
                    })
                },
            )
            .transpose()
    }

    /// Live neighbours reachable from `id` under `relation` (given as either label). Resolves the
    /// label through the registry, follows the canonical edge in the right direction (both
    /// directions for a symmetric relation), and skips soft-deleted neighbours.
    pub fn outgoing(&self, id: MemoryId, relation: &str) -> Result<Vec<MemoryView>, GraphError> {
        let resolved: Option<(String, i64)> = self
            .conn
            .query_row(
                "SELECT name, symmetric FROM relations WHERE name = ?1 OR inverse = ?1",
                params![relation],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
            )
            .optional()
            .map_err(backend)?;
        let Some((canonical, symmetric)) = resolved else {
            return Ok(Vec::new());
        };

        let id = id.0.to_string();
        let neighbour_ids = if symmetric != 0 {
            self.query_ids(
                "SELECT to_id FROM links WHERE from_id = ?1 AND relation = ?2
                 UNION SELECT from_id FROM links WHERE to_id = ?1 AND relation = ?2",
                &id,
                &canonical,
            )?
        } else if relation == canonical {
            self.query_ids(
                "SELECT to_id FROM links WHERE from_id = ?1 AND relation = ?2",
                &id,
                &canonical,
            )?
        } else {
            self.query_ids(
                "SELECT from_id FROM links WHERE to_id = ?1 AND relation = ?2",
                &id,
                &canonical,
            )?
        };

        let mut neighbours = Vec::new();
        for neighbour in neighbour_ids {
            if let Some(memory) = self.memory_by_id(MemoryId(parse_ulid(&neighbour)?))? {
                neighbours.push(memory);
            }
        }
        Ok(neighbours)
    }

    /// All canonical edges touching `id`, with both endpoints live. For inspection and tests; the
    /// agent-facing oriented view is [`Graph::outgoing`].
    pub fn links(&self, id: MemoryId) -> Result<Vec<LinkView>, GraphError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT l.from_id, l.to_id, l.relation FROM links l
                 JOIN memories mf ON mf.id = l.from_id
                 JOIN memories mt ON mt.id = l.to_id
                 WHERE (l.from_id = ?1 OR l.to_id = ?1) AND mf.deleted = 0 AND mt.deleted = 0
                 ORDER BY l.relation, l.to_id",
            )
            .map_err(backend)?;
        let rows = stmt
            .query_map(params![id.0.to_string()], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .map_err(backend)?;

        let mut links = Vec::new();
        for row in rows {
            let (from, to, relation) = row.map_err(backend)?;
            links.push(LinkView {
                from: MemoryId(parse_ulid(&from)?),
                to: MemoryId(parse_ulid(&to)?),
                relation: RelationName::new(relation),
            });
        }
        Ok(links)
    }

    /// Full-text search over name, description, and content, best match first. Over-fetches and
    /// filters soft-deleted memories, mirroring how visibility-aware search will filter hits later.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<MemoryView>, GraphError> {
        let match_query = build_match(query);
        if match_query.is_empty() {
            return Ok(Vec::new());
        }
        let over_fetch = limit.saturating_mul(4).max(limit + 10) as i64;
        let mut stmt = self
            .conn
            .prepare(
                "SELECT memory_id FROM memories_fts WHERE memories_fts MATCH ?1
                 ORDER BY rank LIMIT ?2",
            )
            .map_err(backend)?;
        let rows = stmt
            .query_map(params![match_query, over_fetch], |r| r.get::<_, String>(0))
            .map_err(backend)?;

        let mut hits = Vec::new();
        for row in rows {
            let id = MemoryId(parse_ulid(&row.map_err(backend)?)?);
            if let Some(memory) = self.memory_by_id(id)? {
                hits.push(memory);
                if hits.len() >= limit {
                    break;
                }
            }
        }
        Ok(hits)
    }

    /// Lexical hits with their raw FTS5 bm25 score (more negative is a better match), for the
    /// multi-signal ranker to normalize and blend. Deleted memories are excluded.
    pub fn search_lexical(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<(MemoryId, f32)>, GraphError> {
        let match_query = build_match(query);
        if match_query.is_empty() {
            return Ok(Vec::new());
        }
        let mut stmt = self
            .conn
            .prepare(
                "SELECT f.memory_id, bm25(memories_fts) AS score
                 FROM memories_fts f JOIN memories m ON m.id = f.memory_id
                 WHERE memories_fts MATCH ?1 AND m.deleted = 0
                 ORDER BY score LIMIT ?2",
            )
            .map_err(backend)?;
        let rows = stmt
            .query_map(params![match_query, limit as i64], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?))
            })
            .map_err(backend)?;

        let mut hits = Vec::new();
        for row in rows {
            let (id, score) = row.map_err(backend)?;
            hits.push((MemoryId(parse_ulid(&id)?), score as f32));
        }
        Ok(hits)
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
            volatility: Volatility::parse(&volatility).ok_or_else(|| {
                GraphError::Malformed(format!("unknown volatility {volatility:?}"))
            })?,
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

    /// Resolve a link (asserted under either label) to its stored canonical direction:
    /// `(from_id, to_id, canonical_relation)`. A relation matched by its inverse swaps endpoints;
    /// a symmetric relation orders endpoints so `(a, b)` and `(b, a)` collapse to one edge. An
    /// unregistered relation is stored as given (the Lua layer enforces registration in Stage 4).
    fn canonical_edge(
        &self,
        from: MemoryId,
        to: MemoryId,
        relation: &RelationName,
    ) -> Result<(String, String, String), GraphError> {
        let from = from.0.to_string();
        let to = to.0.to_string();
        let label = relation.as_str();

        let resolved: Option<(String, i64)> = self
            .conn
            .query_row(
                "SELECT name, symmetric FROM relations WHERE name = ?1 OR inverse = ?1",
                params![label],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
            )
            .optional()
            .map_err(backend)?;

        Ok(match resolved {
            None => (from, to, label.to_owned()),
            Some((canonical, symmetric)) if symmetric != 0 => {
                let (lo, hi) = if from <= to { (from, to) } else { (to, from) };
                (lo, hi, canonical)
            }
            Some((canonical, _)) if label == canonical => (from, to, canonical),
            Some((canonical, _)) => (to, from, canonical),
        })
    }

    fn query_ids(&self, sql: &str, id: &str, relation: &str) -> Result<Vec<String>, GraphError> {
        let mut stmt = self.conn.prepare(sql).map_err(backend)?;
        let rows = stmt
            .query_map(params![id, relation], |r| r.get::<_, String>(0))
            .map_err(backend)?;
        let mut ids = Vec::new();
        for row in rows {
            ids.push(row.map_err(backend)?);
        }
        Ok(ids)
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
    Ulid::from_string(text)
        .map_err(|e| GraphError::Malformed(format!("invalid ulid {text:?}: {e}")))
}

fn parse_cardinality(text: &str) -> Result<Cardinality, GraphError> {
    Cardinality::parse(text)
        .ok_or_else(|| GraphError::Malformed(format!("unknown cardinality {text:?}")))
}

/// Build an FTS5 MATCH expression from free text: each whitespace-separated term becomes a quoted
/// phrase (with embedded quotes doubled), joined as an implicit AND. Empty input yields an empty
/// string, which the caller treats as "no query".
fn build_match(query: &str) -> String {
    query
        .split_whitespace()
        .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" ")
}

fn backend(error: rusqlite::Error) -> GraphError {
    GraphError::Backend(error)
}
