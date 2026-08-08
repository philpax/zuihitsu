//! Schema-guard tests: a graph stamped under another schema fingerprint is reset by `guard_schema`
//! (rebuilt from the log by the next `materialize_from`), while a matching stamp preserves the
//! graph, and a read-only open — which cannot make that repair — refuses a graph it did not stamp.
//! The reset cases are driven directly on an in-memory graph, the decision being pure logic over the
//! stored stamp; the read-only cases need a file, because the flag they exercise is an open flag.

use crate::{
    event::{EventPayload, EventSource},
    graph::{Graph, GraphError, schema::schema_fingerprint},
    ids::{MemoryId, Namespace},
    store::{MemoryStore, Store},
    time::Timestamp,
};

fn graph_with_a_row() -> Graph {
    let graph = Graph::open_in_memory().unwrap();
    graph
        .conn
        .execute(
            "INSERT INTO tags (name, description) VALUES ('kept', 'a projected row')",
            [],
        )
        .unwrap();
    graph
}

#[test]
fn a_matching_schema_stamp_keeps_projected_state() {
    let graph = graph_with_a_row();
    graph.guard_schema().unwrap();
    let count: i64 = graph
        .conn
        .query_row("SELECT COUNT(*) FROM tags", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1, "a matching stamp must preserve projected state");
}

#[test]
fn a_graph_stamped_under_another_schema_is_reset() {
    let graph = graph_with_a_row();
    // Simulate a graph written by a build with a different schema: any stored stamp other than the
    // current build's triggers the reset, so a sentinel value stands in for the old build.
    graph
        .conn
        .execute(
            "UPDATE meta SET value = 0 WHERE key = 'schema_fingerprint'",
            [],
        )
        .unwrap();
    graph.guard_schema().unwrap();
    let tags: i64 = graph
        .conn
        .query_row("SELECT COUNT(*) FROM tags", [], |r| r.get(0))
        .unwrap();
    let stamp: i64 = graph
        .conn
        .query_row(
            "SELECT value FROM meta WHERE key = 'schema_fingerprint'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(tags, 0, "a mismatched stamp must reset projected state");
    assert_eq!(
        graph.head().unwrap().0,
        0,
        "a reset graph reports head zero, so replay rebuilds it from the start of the log"
    );
    assert_eq!(
        stamp,
        schema_fingerprint(),
        "the reset restamps the graph under the current schema"
    );
}

/// A read-only open of an existing graph file reads without writing — it checks the schema stamp but
/// runs no DDL and no reset. The file is seeded by a normal `open` + `materialize_from`, then
/// reopened read-only; reads return the materialized state and the head is unchanged.
#[test]
fn open_read_only_reads_without_writing() {
    // The directory guard cleans up the database and its WAL siblings on drop, panic or not.
    let dir = tempfile::tempdir().expect("a temp directory for a graph database");
    let file_path = dir.path().join("graph.sqlite");

    // Seed a graph on disk: create a memory so the graph holds a row and a non-zero head.
    let mut store = MemoryStore::new();
    let memory = MemoryId::generate();
    store
        .append(
            Timestamp::from_millis(1_000),
            EventSource::Agent,
            vec![EventPayload::memory_created(
                memory,
                Namespace::Person.with_name("rowan@direct"),
            )],
        )
        .unwrap();
    {
        let mut graph = Graph::open(&file_path).unwrap();
        graph.materialize_from(&store).unwrap();
        assert_eq!(graph.head().unwrap().0, 1);
    }

    // Reopen read-only: no writes, reads succeed and report the materialized state.
    let graph = Graph::open_read_only(&file_path).unwrap();
    assert_eq!(
        graph.head().unwrap().0,
        1,
        "a read-only open must not reset the graph head"
    );
    // The memory we seeded is readable — the projection survived the read-only reopen.
    let memory_view = graph
        .memory_by_name(Namespace::Person.with_name("rowan@direct"))
        .unwrap()
        .expect("the seeded memory survives a read-only reopen");
    assert_eq!(memory_view.id, memory);

    // A write against the read-only connection must fail — SQLite refuses writes under
    // SQLITE_OPEN_READ_ONLY, proving the flag is effective rather than merely trusting the code path.
    assert!(
        graph
            .conn
            .execute(
                "UPDATE meta SET value = 0 WHERE key = 'schema_fingerprint'",
                []
            )
            .is_err(),
        "a read-only connection must refuse writes"
    );
}

/// A graph stamped under another build's schema is refused by a read-only open rather than read.
/// `guard_schema` would reset and re-materialize such a graph, but that repair is a write, so the
/// read-only path has only the two honest options — refuse, or serve a projection this build did not
/// create. It refuses.
#[test]
fn open_read_only_refuses_a_graph_stamped_under_another_schema() {
    let dir = tempfile::tempdir().expect("a temp directory for a graph database");
    let file_path = dir.path().join("graph.sqlite");

    {
        let graph = Graph::open(&file_path).unwrap();
        graph
            .conn
            .execute(
                "UPDATE meta SET value = ?1 WHERE key = 'schema_fingerprint'",
                [schema_fingerprint().wrapping_add(1)],
            )
            .unwrap();
    }

    // `Graph` is not `Debug`, so the outcome is matched rather than unwrapped.
    match Graph::open_read_only(&file_path) {
        Err(GraphError::SchemaMismatch { .. }) => {}
        Err(error) => panic!("a foreign stamp must read as a schema mismatch, not: {error}"),
        Ok(_) => panic!("a foreign stamp must be refused"),
    }
}

/// An unstamped graph is refused too. A read-write open resets one (recreating empty tables is
/// free), so an unstamped file reaching a read-only open is either empty or older than the stamp,
/// and neither holds a projection worth serving.
#[test]
fn open_read_only_refuses_an_unstamped_graph() {
    let dir = tempfile::tempdir().expect("a temp directory for a graph database");
    let file_path = dir.path().join("graph.sqlite");
    std::fs::write(&file_path, []).expect("an empty database file");

    match Graph::open_read_only(&file_path) {
        Err(GraphError::SchemaMismatch { stored: None, .. }) => {}
        Err(error) => panic!("an empty file must read as unstamped, not: {error}"),
        Ok(_) => panic!("an unstamped graph must be refused"),
    }
}

/// The `guard_schema` reset must stay FK-safe: it drops every table in `sqlite_master` order (no
/// ordering assumption is safe), and with the shared open path's `foreign_keys = ON` a parent
/// drop while its children still hold rows would raise a violation. The reset toggles enforcement
/// off for the drop loop and restores the captured value afterwards; this test proves the whole
/// reset completes, re-stamps, resurrects the FTS shadow tables, and leaves the connection's
/// `foreign_keys` setting exactly as it found it.
#[test]
fn an_fk_safe_reset_completes_and_restores_state() {
    let graph = Graph::open_in_memory().unwrap();
    // Seed rows across FK levels (memory → content entry → conversation), so a parent drop while
    // the children still reference it would trip a violation if enforcement stayed on.
    graph
        .conn
        .execute_batch(
            "INSERT INTO memories (id, name, created_at, class_id, last_content_seq)
             VALUES ('mem:1', 'person/rowan', 1000, 'mem:1', 1);
             INSERT INTO content_entries
                 (entry_id, memory_id, asserted_at, text, told_by, visibility, seq)
             VALUES ('entry:1', 'mem:1', 1000, 'a fact', '\"Agent\"', '\"Public\"', 1);
             INSERT INTO conversations (id, platform, scope_path, context_memory)
             VALUES ('conv:1', 'test', 'room', 'mem:1');",
        )
        .unwrap();

    // The shared open path leaves `foreign_keys` ON before any reset runs.
    let foreign_keys: i64 = graph
        .conn
        .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
        .unwrap();
    assert_eq!(foreign_keys, 1, "the open path must enable foreign keys");

    // Simulate a graph written under another build's schema.
    graph
        .conn
        .execute(
            "UPDATE meta SET value = 0 WHERE key = 'schema_fingerprint'",
            [],
        )
        .unwrap();

    graph.guard_schema().unwrap();

    // (a) The reset completed, dropped the FK-bearing rows, and re-stamped under the current schema.
    let rows: i64 = graph
        .conn
        .query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))
        .unwrap();
    assert_eq!(rows, 0, "the reset must drop projected state");
    let stamp: i64 = graph
        .conn
        .query_row(
            "SELECT value FROM meta WHERE key = 'schema_fingerprint'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        stamp,
        schema_fingerprint(),
        "the reset must re-stamp the graph"
    );

    // (b) The captured `foreign_keys` value is restored — a restoration bug would leave this
    // connection's enforcement permanently off for its whole lifetime.
    let after: i64 = graph
        .conn
        .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
        .unwrap();
    assert_eq!(after, foreign_keys, "the reset must restore foreign_keys");

    // (c) The FTS shadow tables were re-created: the sweep skips `memories_fts_%` by name, so a
    // reset that failed to re-run the DDL batch would leave them gone.
    let fts_shadows: i64 = graph
        .conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type = 'table' AND name LIKE 'memories_fts_%'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        fts_shadows >= 2,
        "the FTS shadow tables must be re-created by the reset (found {fts_shadows})"
    );

    // Enforcement is live again: a child insert without its parent is refused.
    let violation = graph.conn.execute(
        "INSERT INTO content_entries
             (entry_id, memory_id, asserted_at, text, told_by, visibility, seq)
         VALUES ('entry:2', 'mem:nope', 1000, 'x', '\"Agent\"', '\"Public\"', 1)",
        [],
    );
    assert!(
        violation.is_err(),
        "foreign-key enforcement must be live after the reset"
    );

    // And the rebuilt tables are STRICT with the FK clauses declared.
    let memories_ddl: String = graph
        .conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'memories'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(memories_ddl.contains("STRICT"));
    assert!(memories_ddl.contains("FOREIGN KEY (class_id) REFERENCES memories(id)"));
}

/// The boot self-heal path: a graph stamped (or left) under another schema is reset by the open,
/// then rebuilt from the log by the next `materialize_from` — rows and head restored, not lost.
#[test]
fn an_unstamped_graph_rebuilds_from_the_log() {
    let dir = tempfile::tempdir().expect("a temp directory for a graph database");
    let file_path = dir.path().join("graph.sqlite");

    let mut store = MemoryStore::new();
    let memory = MemoryId::generate();
    store
        .append(
            Timestamp::from_millis(1_000),
            EventSource::Agent,
            vec![EventPayload::memory_created(
                memory,
                Namespace::Person.with_name("rowan"),
            )],
        )
        .unwrap();

    // Materialize a stamped graph on disk, then erase its stamp from the file (an older build, or
    // a partial write) behind the graph's back.
    {
        let mut graph = Graph::open(&file_path).unwrap();
        graph.materialize_from(&store).unwrap();
        assert_eq!(graph.head().unwrap().0, 1);
    }
    {
        let conn = rusqlite::Connection::open(&file_path).unwrap();
        conn.execute("DELETE FROM meta WHERE key = 'schema_fingerprint'", [])
            .unwrap();
    }

    // Reopen: the missing stamp triggers the reset (a clean, empty projection at head zero), and
    // the same `materialize_from` that catches up a stale graph rebuilds the rows from the log.
    {
        let mut graph = Graph::open(&file_path).unwrap();
        assert_eq!(
            graph.head().unwrap().0,
            0,
            "an unstamped graph must reset to an empty projection"
        );
        assert_eq!(graph.materialize_from(&store).unwrap(), 1);
        assert_eq!(graph.head().unwrap().0, 1);
        let view = graph
            .memory_by_id(memory)
            .unwrap()
            .expect("the rebuilt graph must restore the memory");
        assert_eq!(view.name.as_str(), "person/rowan");
    }
}

/// The shared pragma set lands on the graph's open paths: `foreign_keys` ON (load-bearing once the
/// schema declares the clauses) and `synchronous` NORMAL on every connection, with a writable
/// file-backed graph additionally in WAL mode. The in-memory graph skips WAL (meaningless there).
#[test]
fn graph_open_carries_the_shared_pragma_defaults() {
    let dir = tempfile::tempdir().expect("a temp directory for a graph database");
    let path = dir.path().join("graph.sqlite");

    let file_graph = Graph::open(&path).unwrap();
    let journal_mode: String = file_graph
        .conn
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        journal_mode, "wal",
        "a file-backed graph must be in WAL mode"
    );
    let foreign_keys: i64 = file_graph
        .conn
        .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        foreign_keys, 1,
        "a file-backed graph must enable foreign keys"
    );
    let synchronous: i64 = file_graph
        .conn
        .query_row("PRAGMA synchronous", [], |row| row.get(0))
        .unwrap();
    assert_eq!(synchronous, 1, "synchronous must be NORMAL (1)");

    let memory_graph = Graph::open_in_memory().unwrap();
    let journal_mode: String = memory_graph
        .conn
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        journal_mode, "memory",
        "an in-memory graph must not be switched to WAL"
    );
    let foreign_keys: i64 = memory_graph
        .conn
        .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        foreign_keys, 1,
        "an in-memory graph must enable foreign keys"
    );
}
