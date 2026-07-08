use std::sync::{Arc, atomic::AtomicI64};

use async_trait::async_trait;

use super::*;
use crate::{
    Instance,
    clock::ManualClock,
    event::EventPayload,
    graph::Graph,
    ids::{ConversationId, MemoryId, Seq, SessionId},
    model::{
        ModelError,
        embed::{Embedder, Embedding},
    },
    store::MemoryStore,
    time::Timestamp,
    vector::{InMemoryVectorIndex, VectorId, VectorRecord},
};

/// An embedder whose `model_id` is configurable, so a test can stand for a model swap; its vectors
/// are constant and never actually compared, only counted and tagged.
struct TaggedEmbedder {
    id: &'static str,
    dims: usize,
}

#[async_trait]
impl Embedder for TaggedEmbedder {
    fn dimensions(&self) -> usize {
        self.dims
    }

    fn model_id(&self) -> &str {
        self.id
    }

    async fn embed(&self, inputs: &[String]) -> Result<Vec<Embedding>, ModelError> {
        Ok(inputs.iter().map(|_| vec![0.1; self.dims]).collect())
    }
}

fn server_over(
    store: MemoryStore,
    vectors: InMemoryVectorIndex,
    model: &'static str,
    dims: usize,
) -> Instance {
    Instance::with_retrieval(
        Box::new(store),
        Graph::open_in_memory().unwrap(),
        Box::new(ManualClock::new(Timestamp::from_millis(2_000))),
        Arc::new(TaggedEmbedder { id: model, dims }),
        Box::new(vectors),
    )
}

#[tokio::test]
async fn a_swap_logs_the_change_and_reembeds_under_the_new_model() {
    let dims = 8;
    // A log with one embeddable description.
    let mut store = MemoryStore::new();
    let mem = MemoryId::generate();
    store
        .append(
            Timestamp::from_millis(1_000),
            vec![EventPayload::memory_description_regenerated(
                mem,
                "an avid climber".to_owned(),
                None,
            )],
        )
        .unwrap();
    // An index that a prior model already built over that log.
    let mut vectors = InMemoryVectorIndex::new();
    vectors
        .upsert(VectorRecord {
            id: VectorId::new("desc:stale"),
            embedding: vec![0.5; dims],
            model_id: "old-model".into(),
        })
        .unwrap();
    vectors.set_cursor(store.head().unwrap()).unwrap();

    let server = server_over(store, vectors, "new-model", dims);
    let reembedded = server.reembed_if_embedding_model_changed().await.unwrap();
    assert!(reembedded, "a model change must trigger a re-embed");

    // The swap is logged, old → new.
    let events = server.engine.store.lock().read_from(Seq::ZERO).unwrap();
    let logged = events.iter().find_map(|event| match &event.payload {
        EventPayload::EmbeddingModelChanged { from, to } => {
            Some((from.to_string(), to.to_string()))
        }
        _ => None,
    });
    assert_eq!(
        logged,
        Some(("old-model".to_owned(), "new-model".to_owned()))
    );

    // The index was cleared of the stale vector and rebuilt under the new model.
    let vectors = server.engine.retrieval.as_ref().unwrap();
    assert_eq!(vectors.vectors.lock().len().unwrap(), 1);
    assert_eq!(
        vectors.vectors.lock().model_id().unwrap().as_deref(),
        Some("new-model")
    );

    // A second boot finds the model unchanged and does nothing.
    assert!(!server.reembed_if_embedding_model_changed().await.unwrap());
}

#[tokio::test]
async fn the_idle_sweep_closes_a_session_once_not_every_tick() {
    // Regression: `flush_and_end` must apply its `SessionEnded` to the graph, not only append it.
    // Otherwise `open_sessions` keeps returning the closed session and the sweep re-closes it every
    // tick — the live-instance "the session ended right after my message" loop.
    let conversation = ConversationId::generate();
    let session = SessionId::generate();
    let mut store = MemoryStore::new();
    store
        .append(
            Timestamp::from_millis(1_000),
            vec![EventPayload::SessionStarted {
                conversation,
                id: session,
                participants: vec![],
                started_at: Timestamp::from_millis(1_000),
                seeded_from_turn: None,
                brief: "brief".to_owned(),
            }],
        )
        .unwrap();

    let mut graph = Graph::open_in_memory().unwrap();
    graph.materialize_from(&store).unwrap();
    let clock = ManualClock::new(Timestamp::from_millis(2_000));
    let server = Instance::new(Box::new(store), graph, Box::new(clock.clone()));
    assert_eq!(
        server.engine.graph.lock().open_sessions().unwrap().len(),
        1,
        "the session starts open"
    );

    // Past the idle gap; the session has no content turns, so the close skips the flush turn and
    // never calls the model.
    clock.advance_millis(7_200_000);
    let model = crate::model::ScriptedModel::new([]);

    assert_eq!(
        server.sweep_idle_sessions(&model).await.unwrap(),
        1,
        "the first sweep closes the idle session"
    );
    assert!(
        server
            .engine
            .graph
            .lock()
            .open_sessions()
            .unwrap()
            .is_empty(),
        "the close must reach the graph so the session reads as ended"
    );
    assert_eq!(
        server.sweep_idle_sessions(&model).await.unwrap(),
        0,
        "a second sweep must not re-close it"
    );

    // The close is recorded exactly once — no repeated `SessionEnded`.
    let ends = server
        .engine
        .store
        .lock()
        .read_from(Seq::ZERO)
        .unwrap()
        .iter()
        .filter(|event| matches!(event.payload, EventPayload::SessionEnded { .. }))
        .count();
    assert_eq!(ends, 1, "the session is ended once, never re-closed");

    // And `flush_and_end` itself is idempotent: invoked again on the now-closed session (as a stale
    // sweep candidate would), it skips rather than appending a second close.
    let stale = Arc::new(OpenSession {
        id: session,
        vm: server.mint_vm(conversation),
        brief: "brief".to_owned(),
        started_at: Timestamp::from_millis(1_000),
        last_activity: AtomicI64::new(1_000),
        start_seq: Seq(1),
        session_start_seq: Seq(1),
    });
    assert!(
        !server
            .flush_and_end(conversation, &stale, &model)
            .await
            .unwrap(),
        "flush_and_end on an already-ended session is a no-op"
    );
    let ends_after = server
        .engine
        .store
        .lock()
        .read_from(Seq::ZERO)
        .unwrap()
        .iter()
        .filter(|event| matches!(event.payload, EventPayload::SessionEnded { .. }))
        .count();
    assert_eq!(ends_after, 1, "no second close was appended");
}

#[tokio::test]
async fn concurrent_closes_of_one_session_record_a_single_end() {
    // A close runs a flush — a model call lasting seconds — before recording `SessionEnded`. In that
    // window the idle sweep and the message-driven recovery path both reach the close for one session.
    // Both hold the conversation's lifecycle lock; serialized through it, the first closes and the
    // second sees the session already ended and skips — exactly one `SessionEnded`, not two.
    let conversation = ConversationId::generate();
    let session = SessionId::generate();
    let mut store = MemoryStore::new();
    store
        .append(
            Timestamp::from_millis(1_000),
            vec![EventPayload::SessionStarted {
                conversation,
                id: session,
                participants: vec![],
                started_at: Timestamp::from_millis(1_000),
                seeded_from_turn: None,
                brief: "brief".to_owned(),
            }],
        )
        .unwrap();
    let mut graph = Graph::open_in_memory().unwrap();
    graph.materialize_from(&store).unwrap();
    let server = Instance::new(
        Box::new(store),
        graph,
        Box::new(ManualClock::new(Timestamp::from_millis(2_000))),
    );

    let open = Arc::new(OpenSession {
        id: session,
        vm: server.mint_vm(conversation),
        brief: "brief".to_owned(),
        started_at: Timestamp::from_millis(1_000),
        last_activity: AtomicI64::new(1_000),
        start_seq: Seq(1),
        session_start_seq: Seq(1),
    });
    let model = crate::model::ScriptedModel::new([]);
    let lifecycle = server.lifecycle_lock(conversation);
    let (a, b) = tokio::join!(
        async {
            let _held = lifecycle.lock().await;
            server.flush_and_end(conversation, &open, &model).await
        },
        async {
            let _held = lifecycle.lock().await;
            server.flush_and_end(conversation, &open, &model).await
        },
    );
    a.unwrap();
    b.unwrap();

    let ends = server
        .engine
        .store
        .lock()
        .read_from(Seq::ZERO)
        .unwrap()
        .iter()
        .filter(|event| matches!(event.payload, EventPayload::SessionEnded { .. }))
        .count();
    assert_eq!(
        ends, 1,
        "two concurrent closes record exactly one SessionEnded"
    );
    assert!(
        !server.engine.graph.lock().session_is_open(session).unwrap(),
        "the session is left ended"
    );
}

#[tokio::test]
async fn an_unchanged_model_is_a_noop_and_an_empty_index_needs_no_migration() {
    let dims = 8;
    // Unchanged model over a populated index: no-op.
    let mut vectors = InMemoryVectorIndex::new();
    vectors
        .upsert(VectorRecord {
            id: VectorId::new("desc:x"),
            embedding: vec![0.5; dims],
            model_id: "same-model".into(),
        })
        .unwrap();
    let server = server_over(MemoryStore::new(), vectors, "same-model", dims);
    assert!(!server.reembed_if_embedding_model_changed().await.unwrap());

    // Empty index (a fresh agent): nothing to migrate, even under a "different" model.
    let fresh = server_over(
        MemoryStore::new(),
        InMemoryVectorIndex::new(),
        "any-model",
        dims,
    );
    assert!(!fresh.reembed_if_embedding_model_changed().await.unwrap());
}

/// The end-to-end path on the real backends across a restart: a log embedded under one model on
/// disk, reopened under another, is detected and re-embedded — exercising the persisted sqlite
/// store, graph, and vec0 index, not just the in-memory fakes.
#[tokio::test]
async fn a_swap_is_detected_and_rebuilt_across_a_real_sqlite_restart() {
    use crate::{ids::Namespace, store::SqliteStore, vector::SqliteVectorIndex};

    let dims = 8;
    let tag = MemoryId::generate().0;
    let dir = std::env::temp_dir();
    let log = dir.join(format!("zuihitsu-emc-log-{tag}.sqlite"));
    let graph_path = dir.join(format!("zuihitsu-emc-graph-{tag}.sqlite"));
    let vecs = dir.join(format!("zuihitsu-emc-vecs-{tag}.sqlite"));

    // Phase 1 — build a log with one embeddable description and index it under model "old", all on
    // disk; then drop the server so the file locks release.
    {
        let mut store = SqliteStore::open(&log).unwrap();
        let mem = MemoryId::generate();
        store
            .append(
                Timestamp::from_millis(1_000),
                vec![
                    EventPayload::memory_created(mem, Namespace::Topic.with_name("x")),
                    EventPayload::memory_description_regenerated(
                        mem,
                        "an avid climber".to_owned(),
                        None,
                    ),
                ],
            )
            .unwrap();
        let server = Instance::with_retrieval(
            Box::new(store),
            Graph::open(&graph_path).unwrap(),
            Box::new(ManualClock::new(Timestamp::from_millis(1_000))),
            Arc::new(TaggedEmbedder { id: "old", dims }),
            Box::new(SqliteVectorIndex::open(&vecs, dims).unwrap()),
        );
        server.index_catch_up().await.unwrap();
        let retrieval = server.engine.retrieval.as_ref().unwrap();
        assert_eq!(
            retrieval.vectors.lock().model_id().unwrap().as_deref(),
            Some("old"),
            "phase 1 should embed under the old model"
        );
    }

    // Phase 2 — restart over the same files under model "new": boot, then the blocking re-embed.
    {
        let vectors = SqliteVectorIndex::open(&vecs, dims).unwrap();
        assert_eq!(
            vectors.model_id().unwrap().as_deref(),
            Some("old"),
            "the persisted index should carry the old model across the restart"
        );
        let mut server = Instance::with_retrieval(
            Box::new(SqliteStore::open(&log).unwrap()),
            Graph::open(&graph_path).unwrap(),
            Box::new(ManualClock::new(Timestamp::from_millis(2_000))),
            Arc::new(TaggedEmbedder { id: "new", dims }),
            Box::new(vectors),
        );
        server.boot().unwrap();
        assert!(server.reembed_if_embedding_model_changed().await.unwrap());

        let events = server.engine.store.lock().read_from(Seq::ZERO).unwrap();
        assert!(
            events.iter().any(|event| matches!(
                &event.payload,
                EventPayload::EmbeddingModelChanged { from, to }
                    if from.as_str() == "old" && to.as_str() == "new"
            )),
            "the swap should be logged old → new"
        );
        let retrieval = server.engine.retrieval.as_ref().unwrap();
        assert_eq!(
            retrieval.vectors.lock().model_id().unwrap().as_deref(),
            Some("new"),
            "the index should be rebuilt under the new model"
        );
        assert_eq!(retrieval.vectors.lock().len().unwrap(), 1);
    }

    for path in [&log, &graph_path, &vecs] {
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
        let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
    }
}
