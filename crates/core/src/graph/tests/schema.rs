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
