//! The reactive projection embeds regenerated descriptions, drops vectors on delete, and can be
//! driven from a full-log rebuild, a subscription drain, or a raw event batch. Uses the
//! deterministic fake embedder, so the same text embeds identically and a query of a memory's own
//! description retrieves it.
use super::{Indexer, ResolvedOp, VectorKey};
use crate::{
    event::{Event, EventPayload, EventSource, Teller, Visibility},
    ids::{EntryId, MemoryId, MemoryName, Namespace},
    model::{
        embed::{CpuEmbedder, Embedder},
        index::{apply_batch, embed_batch},
    },
    store::{MemoryStore, Store},
    time::Timestamp,
    vector::{InMemoryVectorIndex, VectorIndex},
};

const DIMS: usize = 384;

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

    let embedder = CpuEmbedder::try_new().unwrap();
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

    let embedder = CpuEmbedder::try_new().unwrap();
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

    let embedder = CpuEmbedder::try_new().unwrap();
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
    let embedder = CpuEmbedder::try_new().unwrap();
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

// --- EntryContextual tests ---

#[test]
fn vector_key_entry_contextual_round_trips() {
    let entry_id = EntryId::generate();
    let key = VectorKey::EntryContextual(entry_id);
    let vector_id = key.to_vector_id();
    assert_eq!(
        VectorKey::parse(&vector_id),
        Some(VectorKey::EntryContextual(entry_id)),
    );
    // The prefix is `entryctx:`, not `entry:` — the two must not collide.
    assert!(vector_id.0.starts_with("entryctx:"));
    assert!(!vector_id.0.starts_with("entry:"));
}

/// A content-append event for a memory, returning the events to feed `embed_batch` directly.
fn content_appended(id: MemoryId, entry_id: EntryId, text: &str) -> Vec<Event> {
    MemoryStore::new()
        .append(
            at(1),
            EventSource::Agent,
            vec![EventPayload::MemoryContentAppended {
                id,
                entry_id,
                asserted_at: at(1),
                occurred_at: None,
                text: text.to_owned(),
                told_by: Teller::Agent,
                told_in: None,
                visibility: Visibility::Public,
            }],
        )
        .unwrap()
}

#[tokio::test]
async fn embed_batch_produces_both_spaces_with_resolver() {
    let dave = MemoryId::generate();
    let entry = EntryId::generate();
    let events = content_appended(dave, entry, "is a senior developer");

    let embedder = CpuEmbedder::try_new().unwrap();
    let name: MemoryName = Namespace::Person.with_name("dave").into();
    let resolver = move |_id: MemoryId| Some(name.clone());
    let batch = embed_batch(&embedder, &events, Some(&resolver))
        .await
        .unwrap();

    let mut has_entry = false;
    let mut has_entry_contextual = false;
    for op in &batch.ops {
        if let ResolvedOp::Upsert(record) = op {
            if record.id == VectorKey::Entry(entry).to_vector_id() {
                has_entry = true;
            }
            if record.id == VectorKey::EntryContextual(entry).to_vector_id() {
                has_entry_contextual = true;
            }
        }
    }
    assert!(has_entry, "embed_batch should produce an Entry vector");
    assert!(
        has_entry_contextual,
        "embed_batch should produce an EntryContextual vector when a resolver is provided",
    );
}

#[tokio::test]
async fn embed_batch_produces_only_entry_without_resolver() {
    let dave = MemoryId::generate();
    let entry = EntryId::generate();
    let events = content_appended(dave, entry, "is a senior developer");

    let embedder = CpuEmbedder::try_new().unwrap();
    let batch = embed_batch(&embedder, &events, None).await.unwrap();

    let mut has_entry = false;
    let mut has_entry_contextual = false;
    for op in &batch.ops {
        if let ResolvedOp::Upsert(record) = op {
            if record.id == VectorKey::Entry(entry).to_vector_id() {
                has_entry = true;
            }
            if record.id == VectorKey::EntryContextual(entry).to_vector_id() {
                has_entry_contextual = true;
            }
        }
    }
    assert!(has_entry, "embed_batch should produce an Entry vector");
    assert!(
        !has_entry_contextual,
        "embed_batch should not produce an EntryContextual vector without a resolver",
    );
}

#[tokio::test]
async fn embed_batch_gcs_both_spaces_on_supersede() {
    let dave = MemoryId::generate();
    let old = EntryId::generate();
    let new = EntryId::generate();

    let embedder = CpuEmbedder::try_new().unwrap();
    let mut vectors = InMemoryVectorIndex::new();

    // Index the entry with a resolver, so both Entry and EntryContextual vectors are produced.
    let appended = content_appended(dave, old, "old fact");
    let name: MemoryName = Namespace::Person.with_name("dave").into();
    let resolver = move |_id: MemoryId| Some(name.clone());
    let batch = embed_batch(&embedder, &appended, Some(&resolver))
        .await
        .unwrap();
    apply_batch(&mut vectors, batch).unwrap();
    assert_eq!(
        vectors.len().unwrap(),
        2,
        "both Entry and EntryContextual vectors should exist",
    );

    // Now supersede it.
    let superseded = MemoryStore::new()
        .append(
            at(2),
            EventSource::Agent,
            vec![EventPayload::memory_superseded(dave, old, new)],
        )
        .unwrap();
    let batch = embed_batch(&embedder, &superseded, None).await.unwrap();
    apply_batch(&mut vectors, batch).unwrap();

    assert!(
        vectors.is_empty().unwrap(),
        "supersede should remove both Entry and EntryContextual vectors",
    );
}

#[tokio::test]
async fn embed_batch_gcs_both_spaces_on_retract() {
    let dave = MemoryId::generate();
    let entry = EntryId::generate();

    let embedder = CpuEmbedder::try_new().unwrap();
    let mut vectors = InMemoryVectorIndex::new();

    // Index the entry with a resolver, so both Entry and EntryContextual vectors are produced.
    let appended = content_appended(dave, entry, "a fact to retract");
    let name: MemoryName = Namespace::Person.with_name("dave").into();
    let resolver = move |_id: MemoryId| Some(name.clone());
    let batch = embed_batch(&embedder, &appended, Some(&resolver))
        .await
        .unwrap();
    apply_batch(&mut vectors, batch).unwrap();
    assert_eq!(
        vectors.len().unwrap(),
        2,
        "both Entry and EntryContextual vectors should exist",
    );

    // Now retract it.
    let retracted = MemoryStore::new()
        .append(
            at(2),
            EventSource::Agent,
            vec![EventPayload::entry_retracted(
                dave,
                entry,
                "wrong memory",
                None,
            )],
        )
        .unwrap();
    let batch = embed_batch(&embedder, &retracted, None).await.unwrap();
    apply_batch(&mut vectors, batch).unwrap();

    assert!(
        vectors.is_empty().unwrap(),
        "retract should remove both Entry and EntryContextual vectors",
    );
}

#[tokio::test]
async fn embed_batch_gcs_both_spaces_on_consolidated() {
    let dave = MemoryId::generate();
    let source_a = EntryId::generate();
    let source_b = EntryId::generate();
    let replacement = EntryId::generate();

    let embedder = CpuEmbedder::try_new().unwrap();
    let mut vectors = InMemoryVectorIndex::new();

    // Index two source entries with a resolver, so both Entry and EntryContextual vectors
    // are produced for each entry (4 vectors total).
    let appended: Vec<Event> = MemoryStore::new()
        .append(
            at(1),
            EventSource::Agent,
            vec![
                EventPayload::MemoryContentAppended {
                    id: dave,
                    entry_id: source_a,
                    asserted_at: at(1),
                    occurred_at: None,
                    text: "fact A".to_owned(),
                    told_by: Teller::Agent,
                    told_in: None,
                    visibility: Visibility::Public,
                },
                EventPayload::MemoryContentAppended {
                    id: dave,
                    entry_id: source_b,
                    asserted_at: at(1),
                    occurred_at: None,
                    text: "fact B".to_owned(),
                    told_by: Teller::Agent,
                    told_in: None,
                    visibility: Visibility::Public,
                },
            ],
        )
        .unwrap();
    let name: MemoryName = Namespace::Person.with_name("dave").into();
    let resolver = move |_id: MemoryId| Some(name.clone());
    let batch = embed_batch(&embedder, &appended, Some(&resolver))
        .await
        .unwrap();
    apply_batch(&mut vectors, batch).unwrap();
    assert_eq!(
        vectors.len().unwrap(),
        4,
        "both Entry and EntryContextual vectors for both entries should exist",
    );

    // Now consolidate them into a replacement.
    let consolidated = MemoryStore::new()
        .append(
            at(2),
            EventSource::Agent,
            vec![EventPayload::entries_consolidated(
                dave,
                vec![source_a, source_b],
                replacement,
                None,
            )],
        )
        .unwrap();
    let batch = embed_batch(&embedder, &consolidated, None).await.unwrap();
    apply_batch(&mut vectors, batch).unwrap();

    assert!(
        vectors.is_empty().unwrap(),
        "consolidation should remove both Entry and EntryContextual vectors for both sources",
    );
}
