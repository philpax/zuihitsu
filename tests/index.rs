//! Vector-indexer tests: the reactive projection embeds regenerated descriptions, drops vectors on
//! delete, and can be driven from a full-log rebuild, a subscription drain, or a raw event batch.
//! Uses the deterministic fake embedder, so the same text embeds identically and a query of a
//! memory's own description retrieves it.

use zuihitsu::{
    Embedder, FakeEmbedder, InMemoryVectorIndex, Indexer, MemoryId, MemoryName, MemoryStore, Store,
    Timestamp, VectorId, VectorIndex, event::EventPayload,
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
            vec![
                // The indexer ignores the create and reacts only to the description.
                EventPayload::MemoryCreated {
                    id: dave,
                    name: MemoryName::new("person/dave"),
                },
                EventPayload::MemoryDescriptionRegenerated {
                    id: dave,
                    new_text: "An avid rock climber".to_owned(),
                },
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
    assert_eq!(hits[0].id, VectorId::new(dave.0.to_string()));
}

#[tokio::test]
async fn catch_up_resumes_from_the_cursor() {
    let mut store = MemoryStore::new();
    let dave = MemoryId::generate();
    store
        .append(
            at(1),
            vec![EventPayload::MemoryDescriptionRegenerated {
                id: dave,
                new_text: "An avid rock climber".to_owned(),
            }],
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
            vec![EventPayload::MemoryDescriptionRegenerated {
                id: erin,
                new_text: "A tax accountant".to_owned(),
            }],
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
            vec![EventPayload::MemoryDescriptionRegenerated {
                id: dave,
                new_text: "An avid rock climber".to_owned(),
            }],
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
    let key = VectorId::new(dave.0.to_string());

    {
        let mut indexer = Indexer::new(&embedder, &mut vectors);
        // A re-description replaces in place rather than adding a second vector.
        indexer
            .apply(&events(&mut MemoryStore::new(), dave, "old description"))
            .await
            .unwrap();
        indexer
            .apply(&events(&mut MemoryStore::new(), dave, "new description"))
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
        .append(at(2), vec![EventPayload::MemoryDeleted { id: dave }])
        .unwrap();
    Indexer::new(&embedder, &mut vectors)
        .apply(&deletion)
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

/// Commit a description regeneration for `id` and return the resulting events to feed the indexer.
fn events(store: &mut MemoryStore, id: MemoryId, description: &str) -> Vec<zuihitsu::Event> {
    store
        .append(
            at(1),
            vec![EventPayload::MemoryDescriptionRegenerated {
                id,
                new_text: description.to_owned(),
            }],
        )
        .unwrap()
}
