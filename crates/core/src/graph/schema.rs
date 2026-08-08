//! Opening a graph and the projection schema: the DDL batch, the schema-fingerprint guard that
//! resets a graph written under another build's schema (the derived store rebuilds from the log at
//! the next materialisation), and the open paths. The fingerprint is a digest of the DDL itself,
//! so any schema edit moves the stamp with no manually-bumped version to forget.

use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use sha2::{Digest, Sha256};

use crate::{
    db::sqlite_defaults,
    graph::{Graph, GraphError, backend},
};

impl Graph {
    /// Open (creating if absent) a file-backed graph.
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Graph, GraphError> {
        let conn = Connection::open(path).map_err(backend)?;
        sqlite_defaults(&conn, true).map_err(backend)?;
        Self::init(conn)
    }

    /// Open an ephemeral in-memory graph — the no-file-I/O configuration tests use.
    pub fn open_in_memory() -> Result<Graph, GraphError> {
        let conn = Connection::open_in_memory().map_err(backend)?;
        sqlite_defaults(&conn, false).map_err(backend)?;
        Self::init(conn)
    }

    /// Open an existing graph file read-only, taking no lock and running no DDL — a read-only boot
    /// serves the console against an at-rest instance's data without writing or taking the single-writer
    /// lock. No `init` (which runs the schema batch): the file must already exist and be materialized
    /// by a prior live boot. The stamp is still *checked*, because [`Graph::guard_schema`]'s repair is
    /// a write this path cannot make — a graph stamped under another build's schema is refused with
    /// [`GraphError::SchemaMismatch`] rather than read. Checking matters most where the shapes are
    /// compatible: a dropped column fails loudly on the next read anyway, but a schema whose *meaning*
    /// moved with the DDL unchanged in shape would otherwise serve a plausible, wrong projection.
    pub fn open_read_only(path: impl AsRef<std::path::Path>) -> Result<Graph, GraphError> {
        let conn = Connection::open_with_flags(
            path.as_ref(),
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(backend)?;
        sqlite_defaults(&conn, false).map_err(backend)?;
        let graph = Graph { conn };
        graph.check_schema()?;
        Ok(graph)
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
        let stored = self.stored_fingerprint()?;
        if stored != Some(expected) {
            // The reset drops every table and re-runs the DDL batch. With foreign keys ON, dropping
            // a referenced parent while its children still hold rows would raise an FK violation,
            // and the sweep visits tables in `sqlite_master` order, so no ordering is safe. Turn
            // enforcement off for the drop loop and restore the captured value afterwards, or this
            // connection would keep enforcement disabled for its whole lifetime. `defer_foreign_keys`
            // would not work: each `DROP` runs in autocommit, so each deferral resets after the
            // first statement.
            let foreign_keys = self
                .conn
                .query_row("PRAGMA foreign_keys", [], |row| row.get::<_, i64>(0))
                .map_err(backend)?;
            self.conn
                .pragma_update(None, "foreign_keys", "OFF")
                .map_err(backend)?;
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
            // Restore enforcement before the stamp insert: the insert itself needs no FKs, and this
            // ordering keeps the connection's enforcement live from the moment the tables exist —
            // do not move the restore after the stamp write, which would leave a window where a
            // concurrent writer on this connection could insert FK-violating rows.
            self.conn
                .pragma_update(None, "foreign_keys", foreign_keys != 0)
                .map_err(backend)?;
            self.conn
                .execute(
                    "INSERT INTO meta (key, value) VALUES ('schema_fingerprint', ?1)",
                    params![expected],
                )
                .map_err(backend)?;
        }
        Ok(())
    }

    /// [`Graph::guard_schema`]'s check without its repair, for a connection that cannot write. The
    /// unstamped case is a mismatch here too: a read-write open resets an unstamped graph, so an
    /// unstamped file reaching a read-only open is either empty or older than the stamp, and neither
    /// is safe to read as this build's projection.
    fn check_schema(&self) -> Result<(), GraphError> {
        let expected = schema_fingerprint();
        let stored = self.stored_fingerprint()?;
        if stored == Some(expected) {
            Ok(())
        } else {
            Err(GraphError::SchemaMismatch { expected, stored })
        }
    }

    /// The schema stamp this graph file carries, or `None` when it carries none. An absent `meta`
    /// table reads as `None` rather than as a backend failure, because a read-only open may be the
    /// first thing to touch an empty or pre-stamp file; the existence check goes through
    /// `sqlite_master` rather than matching on the "no such table" message text.
    fn stored_fingerprint(&self) -> Result<Option<i64>, GraphError> {
        let meta_exists = self
            .conn
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'meta'",
                [],
                |_| Ok(()),
            )
            .optional()
            .map_err(backend)?
            .is_some();
        if !meta_exists {
            return Ok(None);
        }
        self.conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'schema_fingerprint'",
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(backend)
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
