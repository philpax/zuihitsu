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
