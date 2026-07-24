//! The materialized graph: a pure projection of the event log into queryable SQLite tables.
//!
//! Derived state — it can be dropped and rebuilt from the log at any time without data loss (spec
//! §Storage). This root holds the schema, the open/boot path, and the shared types and helpers; the
//! [`Graph::apply`] materializer lives in [`apply`], and the agent-facing query methods are split by
//! sub-domain across [`memories`], [`occurrences`], [`entries`], [`vocabulary`], [`links`],
//! [`sessions`], and [`search`].

use rusqlite::{Connection, OptionalExtension, params, types::ValueRef};
use sha2::{Digest, Sha256};
use ulid::Ulid;

use crate::{
    db::query_map_into,
    ids::{MemoryId, MemoryName, Seq},
    store::Store,
    time::Timestamp,
    vocabulary::TagName,
};

mod schema;
mod views;

pub use views::*;

mod apply;
mod describe;
mod entries;
mod error;
mod links;
mod memories;
mod occurrences;
mod search;
mod sessions;
#[cfg(test)]
mod tests;
mod vocabulary;

pub use error::GraphError;
pub use search::LexicalHit;

pub struct Graph {
    conn: Connection,
}

impl Graph {
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

    /// Write an atomic checkpoint of the graph to `path` via `VACUUM INTO` (spec §Snapshots). The
    /// `meta.graph_head` rides along in the copy, so the file is a self-describing graph at the head it
    /// was captured at. `path` must not already exist (SQLite refuses to overwrite). The caller is
    /// responsible for capturing at a clean `seq` boundary — no in-flight commit — which holding the
    /// graph lock across this call guarantees (commits take the same lock).
    pub fn snapshot_into(&self, path: &std::path::Path) -> Result<(), GraphError> {
        self.conn
            .execute("VACUUM INTO ?1", params![path.to_string_lossy()])
            .map_err(backend)?;
        Ok(())
    }

    /// A content fingerprint over the graph's logical state — every row of every projected table, in a
    /// canonical order independent of physical layout — so two graphs can be compared for equality: a
    /// snapshot against its source, or a graph rebuilt by replay against the original. Stable across
    /// `VACUUM` (which rebuilds the physical layout but preserves logical content) because it reads only
    /// declared columns in a content order, never implicit rowids. Excludes the derived FTS index,
    /// which is a function of the base tables. `meta` is included, so two graphs match only if they are
    /// at the same `graph_head`.
    pub fn fingerprint(&self) -> Result<String, GraphError> {
        // Every projected table, in a fixed order. The FTS index (`memories_fts`) is derived from
        // these, so it is left out rather than hashing its virtual-table internals.
        const TABLES: &[&str] = &[
            "memories",
            "content_entries",
            "entry_attestations",
            "tags",
            "memory_tags",
            "relations",
            "links",
            "conversations",
            "sessions",
            "session_participants",
            "participant_identities",
            "meta",
        ];
        let mut hasher = Sha256::new();
        for table in TABLES {
            hasher.update(table.as_bytes());
            hasher.update([SEP_TABLE]);
            // Order by every column (by position), so the row sequence is a function of content, not of
            // physical layout — the property that makes the digest VACUUM-stable.
            let column_count = self
                .conn
                .prepare(&format!("SELECT * FROM {table}"))
                .map_err(backend)?
                .column_count();
            let order = (1..=column_count)
                .map(|index| index.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            let mut stmt = self
                .conn
                .prepare(&format!("SELECT * FROM {table} ORDER BY {order}"))
                .map_err(backend)?;
            let mut rows = stmt.query([]).map_err(backend)?;
            while let Some(row) = rows.next().map_err(backend)? {
                for index in 0..column_count {
                    hash_value(&mut hasher, row.get_ref(index).map_err(backend)?);
                }
                hasher.update([SEP_ROW]);
            }
        }
        Ok(hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect())
    }
}

/// The raw memory columns the `memories` SELECT yields; consumed by [`Graph::assemble_memory`].
pub(super) type MemoryColumns = (String, String, String, String, i64);

/// Shared memory-decoding and tag reads used across the sub-domain query modules.
impl Graph {
    /// Assemble a [`MemoryView`] from its raw column tuple, decoding the id and volatility and loading
    /// the memory's tags. Shared by every memory query that selects the standard `memories` columns.
    fn assemble_memory(&self, columns: MemoryColumns) -> Result<MemoryView, GraphError> {
        let (id, name, description, volatility, created_at) = columns;
        Ok(MemoryView {
            id: MemoryId(parse_ulid(&id)?),
            name: MemoryName::new(name),
            description,
            volatility: volatility.parse().map_err(|()| {
                GraphError::Malformed(format!("unknown volatility {volatility:?}"))
            })?,
            created_at: timestamp_column(created_at, "created_at")?,
            tags: self.tags_of(&id)?,
        })
    }

    fn tags_of(&self, memory_id: &str) -> Result<Vec<TagName>, GraphError> {
        let stmt = self
            .conn
            .prepare("SELECT tag FROM memory_tags WHERE memory_id = ?1 ORDER BY tag")?;
        query_map_into(stmt, params![memory_id], |row| {
            let tag: String = row.get(0)?;
            Ok(TagName::new(&tag))
        })
    }
}

/// Row and table separators in the fingerprint stream, so distinct row/table boundaries cannot be
/// forged by content (e.g. two short fields colliding with one longer field).
const SEP_ROW: u8 = 0xFF;
const SEP_TABLE: u8 = 0xFE;

/// Feed one SQLite value into the fingerprint hasher, tagged by type and length-prefixed, so values of
/// different types or lengths can never produce the same byte stream.
fn hash_value(hasher: &mut Sha256, value: ValueRef<'_>) {
    match value {
        ValueRef::Null => hasher.update([0]),
        ValueRef::Integer(int) => {
            hasher.update([1]);
            hasher.update(int.to_le_bytes());
        }
        ValueRef::Real(real) => {
            hasher.update([2]);
            hasher.update(real.to_le_bytes());
        }
        ValueRef::Text(text) => {
            hasher.update([3]);
            hasher.update((text.len() as u64).to_le_bytes());
            hasher.update(text);
        }
        ValueRef::Blob(blob) => {
            hasher.update([4]);
            hasher.update((blob.len() as u64).to_le_bytes());
            hasher.update(blob);
        }
    }
}

fn parse_ulid(text: &str) -> Result<Ulid, GraphError> {
    Ulid::from_string(text)
        .map_err(|e| GraphError::Malformed(format!("invalid ulid {text:?}: {e}")))
}

/// Decode a `column`'s epoch-millisecond value into a [`Timestamp`], or a [`GraphError::Malformed`]
/// if it falls outside jiff's representable range — the projection is derived from the event log, so
/// an out-of-range value here means the log itself carries a value the wire's [`Timestamp`] serde
/// impl would have rejected, not a value this read can silently coerce.
pub(super) fn timestamp_column(millis: i64, column: &str) -> Result<Timestamp, GraphError> {
    Timestamp::try_from_millis(millis).ok_or_else(|| {
        GraphError::Malformed(format!(
            "{column} {millis} milliseconds since the Unix epoch is outside the representable range"
        ))
    })
}

pub(super) fn backend(error: rusqlite::Error) -> GraphError {
    GraphError::Backend(error)
}
