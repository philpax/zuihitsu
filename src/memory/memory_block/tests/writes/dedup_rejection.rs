//! The dedup check's rejection path: a contextual near-duplicate is caught against the `EntryContextual`
//! space, and a fact founded as another teller's confidence never captures an independent speaker's
//! statement, though the confiding teller's own repeat still does.

use std::sync::Arc;

use crate::{
    clock::ManualClock,
    event::{EventPayload, EventSource, Teller, Visibility},
    graph::Graph,
    ids::{EntryId, MemoryId, MemoryName, Namespace},
    memory::memory_block::tests::writes::{
        AppendOptions, Authority, MemoryError, block_with_retrieval,
    },
    model::embed::{CpuEmbedder, Embedder},
    store::{MemoryStore, Store},
    time::Timestamp,
    vector::{InMemoryVectorIndex, VectorIndex, VectorRecord},
};

#[tokio::test]
async fn append_dedup_rejects_contextual_duplicate() {
    // The dedup check should search the EntryContextual space, not the Entry space. Seed an
    // EntryContextual vector for an existing live entry, then call append_dedup with the same
    // contextual embedding — the same text embeds to the same vector, and the dedup check
    // should reject the duplicate.
    let embedder: Arc<dyn Embedder> = CpuEmbedder::shared();

    let dave_name: MemoryName = Namespace::Person.with_name("dave").into();
    let dave: MemoryId = MemoryId::generate();
    let existing_entry = EntryId::generate();
    let existing_text = "is a senior developer";

    // Build a graph with the memory and its entry.
    let mut store = MemoryStore::new();
    store
        .append(
            Timestamp::from_millis(1_000),
            EventSource::Agent,
            vec![
                EventPayload::memory_created(dave, dave_name.clone()),
                EventPayload::MemoryContentAppended {
                    id: dave,
                    entry_id: existing_entry,
                    asserted_at: Timestamp::from_millis(1_000),
                    occurred_at: None,
                    text: existing_text.to_owned(),
                    told_by: Teller::Agent,
                    told_in: None,
                    visibility: Visibility::Public,
                },
            ],
        )
        .unwrap();
    let mut graph = Graph::open_in_memory().unwrap();
    graph.materialize_from(&store).unwrap();

    // Seed the EntryContextual vector with the contextual embedding of the existing entry.
    let mut vectors = InMemoryVectorIndex::new();
    let contextual_text = crate::model::embed::contextual_text(dave_name.as_str(), existing_text);
    let contextual_embedding = embedder.embed(&[contextual_text]).await.unwrap().remove(0);
    vectors
        .upsert(VectorRecord {
            id: crate::model::index::VectorKey::EntryContextual(existing_entry).to_vector_id(),
            embedding: contextual_embedding.clone(),
            model_id: embedder.model_id().into(),
        })
        .unwrap();

    let clock = ManualClock::new(Timestamp::from_millis(2_000));
    let mut block = block_with_retrieval(
        graph,
        clock,
        Teller::Agent,
        Authority::Platform,
        embedder,
        Box::new(vectors),
    );

    // The same contextual embedding is passed as the dedup embedding — the dedup check should
    // find the seeded EntryContextual vector above the threshold and reject the append.
    let error = block
        .append_dedup(
            dave,
            "is a senior developer",
            AppendOptions::default(),
            Some(&contextual_embedding),
        )
        .unwrap_err();
    assert!(
        matches!(error, MemoryError::DuplicateEntry { .. }),
        "expected DuplicateEntry, got {error:?}",
    );
}

#[tokio::test]
async fn append_dedup_ignores_another_tellers_confidence() {
    // A near-duplicate check must not capture against an entry founded as another teller's
    // confidence: the incoming speaker was never told it, so their independent statement appends
    // normally rather than being rejected against — or shown a snippet of — a fact confided by
    // someone else. The confiding teller's own repeat still captures.
    let embedder: Arc<dyn Embedder> = CpuEmbedder::shared();

    let dave_name: MemoryName = Namespace::Person.with_name("dave").into();
    let dave: MemoryId = MemoryId::generate();
    let confider: MemoryId = MemoryId::generate();
    let confided_entry = EntryId::generate();
    let confided_text = "is quietly job hunting";

    let mut store = MemoryStore::new();
    store
        .append(
            Timestamp::from_millis(1_000),
            EventSource::Agent,
            vec![
                EventPayload::memory_created(dave, dave_name.clone()),
                EventPayload::memory_created(confider, Namespace::Person.with_name("erin")),
                EventPayload::MemoryContentAppended {
                    id: dave,
                    entry_id: confided_entry,
                    asserted_at: Timestamp::from_millis(1_000),
                    occurred_at: None,
                    text: confided_text.to_owned(),
                    told_by: Teller::Participant(confider),
                    told_in: None,
                    visibility: Visibility::PrivateToTeller,
                },
            ],
        )
        .unwrap();
    let mut graph = Graph::open_in_memory().unwrap();
    graph.materialize_from(&store).unwrap();

    let mut vectors = InMemoryVectorIndex::new();
    let contextual_text = crate::model::embed::contextual_text(dave_name.as_str(), confided_text);
    let contextual_embedding = embedder.embed(&[contextual_text]).await.unwrap().remove(0);
    vectors
        .upsert(VectorRecord {
            id: crate::model::index::VectorKey::EntryContextual(confided_entry).to_vector_id(),
            embedding: contextual_embedding.clone(),
            model_id: embedder.model_id().into(),
        })
        .unwrap();

    // A different speaker's independent statement of the same fact appends normally.
    let other_speaker: MemoryId = MemoryId::generate();
    let clock = ManualClock::new(Timestamp::from_millis(2_000));
    let mut block = block_with_retrieval(
        graph,
        clock,
        Teller::Participant(other_speaker),
        Authority::Platform,
        embedder,
        Box::new(vectors),
    );
    block
        .append_dedup(
            dave,
            "is quietly job hunting",
            AppendOptions::default(),
            Some(&contextual_embedding),
        )
        .expect("another teller's confidence must not capture an independent statement");

    // The confiding teller's own repeat still captures as a duplicate.
    let error = block
        .append_dedup(
            dave,
            "is quietly job hunting",
            AppendOptions {
                told_by: Some(Teller::Participant(confider)),
                ..AppendOptions::default()
            },
            Some(&contextual_embedding),
        )
        .unwrap_err();
    assert!(
        matches!(error, MemoryError::DuplicateEntry { .. }),
        "expected DuplicateEntry for the confiding teller's own repeat, got {error:?}",
    );
}
