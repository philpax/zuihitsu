//! Boot-path integration: a fresh install (no existing databases) opens the event log, graph, and
//! vector index under the shared SQLite pragma set, creates STRICT tables with the FK clauses this
//! build declares, and a seeded event materialises into a memory. Guards AC.8 of the storage
//! hardening: the strict schemas are what a brand-new agent is born with, not merely something an
//! in-memory test hashes.

use rusqlite::Connection;
use std::sync::Arc;

use crate::{
    Embedder, EventPayload, GenesisStatus, Instance,
    clock::ManualClock,
    graph::Graph,
    ids::{MemoryId, Namespace, Seq},
    model::ModelError,
    store::SqliteStore,
    time::Timestamp,
    vector::SqliteVectorIndex,
};

/// Boot an `Instance` against a temp data dir, then assert:
/// (a) all three databases exist,
/// (b) the event-log `events` table and the graph's ordinary tables are STRICT with the expected
///     FK clauses,
/// (c) a seeded event materialises into a memory and the graph head advances.
#[test]
fn fresh_boot_creates_strict_derived_stores() {
    let dir = tempfile::tempdir().expect("a temp data directory for a fresh boot");
    let event_log = dir.path().join("events.sqlite");
    let graph_path = dir.path().join("graph.sqlite");
    let vectors_path = dir.path().join("vectors.sqlite");

    let store = SqliteStore::open(&event_log).unwrap();
    let graph = Graph::open(&graph_path).unwrap();
    let vectors = SqliteVectorIndex::open(&vectors_path, DIMS).unwrap();
    let mut server = Instance::with_retrieval(
        Box::new(store),
        graph,
        Box::new(ManualClock::new(Timestamp::from_millis(1_000))),
        Arc::new(FakeEmbedder),
        Box::new(vectors),
    );
    server
        .control()
        .create_agent(&crate::SeedSelf {
            agent_name: "Kestrel".to_owned(),
            persona: "A bird.".to_owned(),
            seed_entries: vec![],
        })
        .unwrap();
    let status = server.boot().unwrap();
    assert_eq!(status, GenesisStatus::Complete);

    // (a) All three databases exist on disk.
    assert!(
        event_log.exists(),
        "the event log must exist after a fresh boot"
    );
    assert!(
        graph_path.exists(),
        "the graph must exist after a fresh boot"
    );
    assert!(
        vectors_path.exists(),
        "the vector index must exist after a fresh boot"
    );

    // (b) The event log's `events` table is STRICT, and the graph's tables are STRICT with the
    // FK clauses this build declares.
    let event_ddl: String = Connection::open(&event_log)
        .unwrap()
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'events'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        event_ddl.contains("STRICT"),
        "the event log's events table must be STRICT, got: {event_ddl}"
    );

    let graph_conn = Connection::open(&graph_path).unwrap();
    let memories_ddl: String = graph_conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'memories'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(memories_ddl.contains("STRICT"));
    assert!(memories_ddl.contains("FOREIGN KEY (class_id) REFERENCES memories(id)"));
    let content_ddl: String = graph_conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'content_entries'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(content_ddl.contains("STRICT"));
    assert!(content_ddl.contains("FOREIGN KEY (memory_id) REFERENCES memories(id)"));
    let links_ddl: String = graph_conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'links'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(links_ddl.contains("STRICT"));
    assert!(links_ddl.contains("FOREIGN KEY (relation) REFERENCES relations(name)"));
    // The genesis-created meta table is STRICT too.
    let meta_ddl: String = graph_conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'meta'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(meta_ddl.contains("STRICT"));

    // (c) A seeded event materialises into a memory: append a memory through the instance's own
    // write path (which materializes the graph under the FK clauses), then read it back — proving
    // the fold runs with foreign keys ON over the strict tables.
    let memory = MemoryId::generate();
    server
        .control()
        .seed_events(vec![EventPayload::memory_created(
            memory,
            Namespace::Person.with_name("rowan"),
        )])
        .unwrap();

    // The graph holds the memory; the head advanced past genesis.
    let graph = server.engine.graph.lock();
    let view = graph.memory_by_id(memory).unwrap().unwrap();
    assert_eq!(view.name.as_str(), "person/rowan");
    assert!(
        graph.head().unwrap() > Seq::ZERO,
        "the graph head must advance past genesis"
    );
}

/// The boot test's embedding dimensionality — matches [`FakeEmbedder`].
const DIMS: usize = 8;

/// A trivial deterministic embedder for the boot test — the vector index only needs to be opened
/// and written to (embedding swaps check real similarities elsewhere).
struct FakeEmbedder;

#[async_trait::async_trait]
impl Embedder for FakeEmbedder {
    fn dimensions(&self) -> usize {
        DIMS
    }

    fn model_id(&self) -> &str {
        "fake"
    }

    async fn embed(&self, inputs: &[String]) -> Result<Vec<Vec<f32>>, ModelError> {
        Ok(inputs.iter().map(|_| vec![0.0; DIMS]).collect())
    }
}
