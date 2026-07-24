//! Opening a graph and the projection schema: the DDL batch, the schema-fingerprint guard that
//! resets a graph written under another build's schema (the derived store rebuilds from the log at
//! the next materialisation), and the open paths. The fingerprint is a digest of the DDL itself,
//! so any schema edit moves the stamp with no manually-bumped version to forget.

use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};

use crate::graph::{Graph, GraphError, backend};

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
        conn.execute_batch(Self::SCHEMA_SQL).map_err(backend)?;
        let graph = Graph { conn };
        graph.guard_schema()?;
        Ok(graph)
    }

    /// Reset the graph unless its stored schema fingerprint matches this build's, so a binary whose
    /// projection schema has moved never reads or writes a table shape it did not create (an added
    /// column would otherwise surface as a runtime `no such column` error deep in a read). The graph
    /// is a derived store — `materialize_from` rebuilds a reset graph from the event log — so the
    /// reset trades one full replay for schema correctness and loses no logical state. A graph
    /// without a stamp (fresh, or written by a build predating the stamp) resets too: recreating
    /// empty tables is free, and it is the only safe reading of an unstamped file.
    pub(super) fn guard_schema(&self) -> Result<(), GraphError> {
        let expected = schema_fingerprint();
        let stored: Option<i64> = self
            .conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'schema_fingerprint'",
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(backend)?;
        if stored != Some(expected) {
            // The FTS shadow tables (`memories_fts_*`) drop with their virtual table, so they are
            // excluded from the sweep rather than dropped twice.
            let tables: Vec<String> = self
                .conn
                .prepare(
                    "SELECT name FROM sqlite_master
                     WHERE type = 'table' AND name NOT LIKE 'memories_fts_%'",
                )
                .map_err(backend)?
                .query_map([], |r| r.get(0))
                .map_err(backend)?
                .collect::<Result<_, _>>()
                .map_err(backend)?;
            for table in tables {
                self.conn
                    .execute_batch(&format!("DROP TABLE IF EXISTS \"{table}\""))
                    .map_err(backend)?;
            }
            self.conn.execute_batch(Self::SCHEMA_SQL).map_err(backend)?;
            self.conn
                .execute(
                    "INSERT INTO meta (key, value) VALUES ('schema_fingerprint', ?1)",
                    params![expected],
                )
                .map_err(backend)?;
        }
        Ok(())
    }

    /// The projection schema, one idempotent DDL batch, included from `schema.sql` beside this
    /// module (plain SQL: highlighted, diffable, and interpolation-free). Also the input to
    /// `schema_fingerprint`, so any edit to the file moves the stamp `guard_schema` checks, with no
    /// manually-bumped version to forget.
    const SCHEMA_SQL: &'static str = include_str!("schema.sql");
}

/// The stamp `guard_schema` compares: the leading eight bytes of a SHA-256 over the schema DDL,
/// stored in `meta` as an integer. A digest of the DDL text itself, so the stamp is a pure function
/// of the schema with no versioning discipline to uphold.
pub(super) fn schema_fingerprint() -> i64 {
    let digest = Sha256::digest(Graph::SCHEMA_SQL.as_bytes());
    i64::from_le_bytes(
        digest[..8]
            .try_into()
            .expect("a SHA-256 digest holds at least eight bytes"),
    )
}
