//! Schema-guard tests: a graph stamped under another schema fingerprint is reset by `guard_schema`
//! (rebuilt from the log by the next `materialize_from`), while a matching stamp preserves the
//! graph. The guard is driven directly on an in-memory graph — the reset decision is pure logic
//! over the stored stamp, and file persistence between opens is SQLite's property, not ours.

use crate::graph::{Graph, schema::schema_fingerprint};

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

/// A read-only open of an existing graph file reads without writing — no `guard_schema`, no DDL.
/// The file is seeded by a normal `open` + `materialize_from`, then reopened read-only; reads
/// return the materialized state and the head is unchanged (no reset).
#[test]
fn open_read_only_reads_without_writing() {
    use crate::{
        event::{EventPayload, EventSource},
        store::{MemoryStore, Store},
        time::Timestamp,
    };

    let path = super::temp_graph_path();
    // `Graph::open` creates the file; `temp_graph_path` gave us a path that does not yet exist
    // (NamedTempFile creates then detaches), so open it at the path.
    let file_path = path.keep().expect("keep the temp file");

    // Seed a graph on disk: create a memory so the graph holds a row and a non-zero head.
    let mut store = MemoryStore::new();
    let memory = crate::ids::MemoryId::generate();
    store
        .append(
            Timestamp::from_millis(1_000),
            EventSource::Agent,
            vec![EventPayload::memory_created(
                memory,
                crate::ids::Namespace::Person.with_name("rowan@direct"),
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
        .memory_by_name(crate::ids::Namespace::Person.with_name("rowan@direct"))
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

    let _ = std::fs::remove_file(&file_path);
}
