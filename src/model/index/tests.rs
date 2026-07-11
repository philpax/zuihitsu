//! The reactive projection embeds regenerated descriptions, drops vectors on delete, and can be
//! driven from a full-log rebuild, a subscription drain, or a raw event batch. Uses the
//! deterministic fake embedder, so the same text embeds identically and a query of a memory's own
//! description retrieves it.
use super::{Indexer, VectorKey};
use crate::{
    event::{Event, EventPayload, EventSource},
    ids::{MemoryId, Namespace},
    model::embed::{Embedder, FakeEmbedder},
    store::{MemoryStore, Store},
    time::Timestamp,
    vector::{InMemoryVectorIndex, VectorIndex},
};

const DIMS: usize = 16;

fn at(ms: i64) -> Timestamp {
    Timestamp::from_millis(ms)
}

#[tokio::test]
async fn catch_up_embeds_each_memorys_description() {
    let mut store = MemoryStore::new();
    let dave = MemoryId::generate();
    store
        .append(
            at(1),
            EventSource::Agent,
            vec![
                // The indexer ignores the create and reacts only to the description.
                EventPayload::memory_created(dave, Namespace::Person.with_name("dave")),
                EventPayload::memory_description_regenerated(
                    dave,
                    "An avid rock climber".to_owned(),
                    None,
                ),
            ],
        )
        .unwrap();

    let embedder = FakeEmbedder::new(DIMS);
    let mut vectors = InMemoryVectorIndex::new();
    Indexer::new(&embedder, &mut vectors)
        .catch_up(&store)
        .await
        .unwrap();

    assert_eq!(vectors.len().unwrap(), 1);
    // Querying Dave's own description retrieves his vector, keyed by memory id.
    let query = embedder
        .embed(&["An avid rock climber".to_owned()])
        .await
        .unwrap()
        .remove(0);
    let hits = vectors.search(&query, 1).unwrap();
    assert_eq!(hits[0].id, VectorKey::Description(dave).to_vector_id());
}

#[tokio::test]
async fn catch_up_resumes_from_the_cursor() {
    let mut store = MemoryStore::new();
    let dave = MemoryId::generate();
    store
        .append(
            at(1),
            EventSource::Agent,
            vec![EventPayload::memory_description_regenerated(
                dave,
                "An avid rock climber".to_owned(),
                None,
            )],
        )
        .unwrap();

    let embedder = FakeEmbedder::new(DIMS);
    let mut vectors = InMemoryVectorIndex::new();

    // First catch-up processes the one event and advances the cursor to its seq.
    assert_eq!(
        Indexer::new(&embedder, &mut vectors)
            .catch_up(&store)
            .await
            .unwrap(),
        1
    );
    assert_eq!(vectors.cursor().unwrap(), store.head().unwrap());

    // With nothing new in the log, a second catch-up is a no-op (doesn't re-embed).
    assert_eq!(
        Indexer::new(&embedder, &mut vectors)
            .catch_up(&store)
            .await
            .unwrap(),
        0
    );

    // A new event is the only thing the next catch-up processes.
    let erin = MemoryId::generate();
    store
        .append(
            at(2),
            EventSource::Agent,
            vec![EventPayload::memory_description_regenerated(
                erin,
                "A tax accountant".to_owned(),
                None,
            )],
        )
        .unwrap();
    assert_eq!(
        Indexer::new(&embedder, &mut vectors)
            .catch_up(&store)
            .await
            .unwrap(),
        1
    );
    assert_eq!(vectors.len().unwrap(), 2);
    assert_eq!(vectors.cursor().unwrap(), store.head().unwrap());
}

#[tokio::test]
async fn drain_indexes_subscribed_events() {
    let mut store = MemoryStore::new();
    let subscription = store.subscribe();
    let dave = MemoryId::generate();
    store
        .append(
            at(1),
            EventSource::Agent,
            vec![EventPayload::memory_description_regenerated(
                dave,
                "An avid rock climber".to_owned(),
                None,
            )],
        )
        .unwrap();

    let embedder = FakeEmbedder::new(DIMS);
    let mut vectors = InMemoryVectorIndex::new();
    let processed = Indexer::new(&embedder, &mut vectors)
        .drain(&subscription)
        .await
        .unwrap();

    assert_eq!(processed, 1);
    assert_eq!(vectors.len().unwrap(), 1);
}

#[tokio::test]
async fn a_later_regeneration_replaces_and_a_delete_removes() {
    let dave = MemoryId::generate();
    let embedder = FakeEmbedder::new(DIMS);
    let mut vectors = InMemoryVectorIndex::new();
    let key = VectorKey::Description(dave).to_vector_id();

    {
        let mut indexer = Indexer::new(&embedder, &mut vectors);
        // A re-description replaces in place rather than adding a second vector.
        indexer
            .index_batch(&events(&mut MemoryStore::new(), dave, "old description"))
            .await
            .unwrap();
        indexer
            .index_batch(&events(&mut MemoryStore::new(), dave, "new description"))
            .await
            .unwrap();
    }
    assert_eq!(vectors.len().unwrap(), 1);
    let new_query = embedder
        .embed(&["new description".to_owned()])
        .await
        .unwrap()
        .remove(0);
    assert!((vectors.search(&new_query, 1).unwrap()[0].score - 1.0).abs() < 1e-3);

    // A delete drops the vector.
    let mut store = MemoryStore::new();
    let deletion = store
        .append(
            at(2),
            EventSource::Agent,
            vec![EventPayload::memory_deleted(dave)],
        )
        .unwrap();
    Indexer::new(&embedder, &mut vectors)
        .index_batch(&deletion)
        .await
        .unwrap();
    assert!(vectors.is_empty().unwrap());
    assert!(
        vectors
            .search(&new_query, 5)
            .unwrap()
            .iter()
            .all(|hit| hit.id != key)
    );
}

#[tokio::test]
async fn a_blank_description_is_skipped_without_embedding() {
    // A real embedding endpoint rejects an empty input, so a blank description (or entry) must
    // never reach the embedder. `StrictEmbedder` panics if it does; the indexer should skip it
    // and produce no vector.
    let mut store = MemoryStore::new();
    let ghost = MemoryId::generate();
    store
        .append(
            at(1),
            EventSource::Agent,
            vec![EventPayload::memory_description_regenerated(
                ghost,
                "   ".to_owned(),
                None,
            )],
        )
        .unwrap();

    let embedder = StrictEmbedder;
    let mut vectors = InMemoryVectorIndex::new();
    let processed = Indexer::new(&embedder, &mut vectors)
        .catch_up(&store)
        .await
        .unwrap();

    // The event is processed (the cursor advances) but yields no vector.
    assert_eq!(processed, 1);
    assert!(vectors.is_empty().unwrap());
    assert_eq!(vectors.cursor().unwrap(), store.head().unwrap());
}

/// An embedder that refuses blank input, mirroring a real endpoint — so a test fails loudly if
/// the indexer ever forwards an empty string to embed.
struct StrictEmbedder;

#[async_trait::async_trait]
impl Embedder for StrictEmbedder {
    fn dimensions(&self) -> usize {
        DIMS
    }

    fn model_id(&self) -> &str {
        "strict-test-embedder"
    }

    async fn embed(
        &self,
        inputs: &[String],
    ) -> Result<Vec<crate::model::embed::Embedding>, crate::model::ModelError> {
        assert!(
            inputs.iter().all(|text| !text.trim().is_empty()),
            "the indexer forwarded a blank text to the embedder"
        );
        Ok(inputs.iter().map(|_| vec![0.0; DIMS]).collect())
    }
}

/// Commit a description regeneration for `id` and return the resulting events to feed the indexer.
fn events(store: &mut MemoryStore, id: MemoryId, description: &str) -> Vec<Event> {
    store
        .append(
            at(1),
            EventSource::Agent,
            vec![EventPayload::memory_description_regenerated(
                id,
                description.to_owned(),
                None,
            )],
        )
        .unwrap()
}
