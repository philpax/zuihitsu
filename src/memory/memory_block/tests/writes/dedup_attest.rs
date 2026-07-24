//! The dedup check's auto-attest capture matrix: a new teller's corroboration of an all-audience fact
//! folds in as an `EntryAttested` rather than a second entry — recording distinct phrasing, the private
//! confirmation, the idempotent re-attest no-op, the same-teller error, and the `distinct_from` skip.

use std::sync::Arc;

use crate::{
    clock::ManualClock,
    event::{EventPayload, EventSource, Teller, Visibility},
    graph::Graph,
    ids::{EntryId, MemoryId, MemoryName, Namespace},
    memory::memory_block::{
        AppendOutcome, EntrySelector,
        tests::writes::{
            AppendOptions, Authority, MemoryError, VisibilityChoice, attestations,
            block_with_retrieval,
        },
    },
    model::embed::{CpuEmbedder, Embedder},
    store::{MemoryStore, Store},
    time::Timestamp,
    vector::{InMemoryVectorIndex, VectorIndex, VectorRecord},
};

/// Seed a graph and its contextual vector for one live entry on `person/dave`, returning the graph,
/// the seeded vector index, the embedder, and the entry's contextual embedding — the fixture the
/// auto-attest capture-matrix tests drive a `block_with_retrieval` over.
async fn seeded_entry(
    dave: MemoryId,
    entry_id: EntryId,
    text: &str,
    told_by: Teller,
    visibility: Visibility,
) -> (Graph, InMemoryVectorIndex, Arc<dyn Embedder>, Vec<f32>) {
    let embedder: Arc<dyn Embedder> = CpuEmbedder::shared();
    let dave_name: MemoryName = Namespace::Person.with_name("dave").into();
    let mut store = MemoryStore::new();
    store
        .append(
            Timestamp::from_millis(1_000),
            EventSource::Agent,
            vec![
                EventPayload::memory_created(dave, dave_name.clone()),
                EventPayload::MemoryContentAppended {
                    id: dave,
                    entry_id,
                    asserted_at: Timestamp::from_millis(1_000),
                    occurred_at: None,
                    text: text.to_owned(),
                    told_by,
                    told_in: None,
                    visibility,
                },
            ],
        )
        .unwrap();
    let mut graph = Graph::open_in_memory().unwrap();
    graph.materialize_from(&store).unwrap();

    let mut vectors = InMemoryVectorIndex::new();
    let contextual = crate::model::embed::contextual_text(dave_name.as_str(), text);
    let embedding = embedder.embed(&[contextual]).await.unwrap().remove(0);
    vectors
        .upsert(VectorRecord {
            id: crate::model::index::VectorKey::EntryContextual(entry_id).to_vector_id(),
            embedding: embedding.clone(),
            model_id: embedder.model_id().into(),
        })
        .unwrap();
    (graph, vectors, embedder, embedding)
}

#[tokio::test]
async fn append_dedup_auto_attests_a_public_fact_for_a_new_teller() {
    // A different teller's independent statement of an all-audience fact is folded in as their
    // corroboration: the append succeeds, returns the existing entry's id, and buffers one
    // EntryAttested under the incoming teller — no second content entry.
    let dave = MemoryId::generate();
    let entry_id = EntryId::generate();
    let (graph, vectors, embedder, embedding) = seeded_entry(
        dave,
        entry_id,
        "is a senior developer",
        Teller::Agent,
        Visibility::Public,
    )
    .await;

    let speaker = MemoryId::generate();
    let clock = ManualClock::new(Timestamp::from_millis(2_000));
    let mut block = block_with_retrieval(
        graph,
        clock,
        Teller::Participant(speaker),
        Authority::Platform,
        embedder,
        Box::new(vectors),
    );
    let outcome = block
        .append_dedup(
            dave,
            "is a senior developer",
            AppendOptions {
                visibility: Some(VisibilityChoice::Public),
                ..AppendOptions::default()
            },
            Some(&embedding),
        )
        .unwrap();
    let AppendOutcome::Corroborated(corroboration) = outcome else {
        panic!("expected a corroboration, got a fresh append");
    };
    assert_eq!(
        corroboration.entry, entry_id,
        "the corroboration hands back the existing entry's id"
    );

    let events = block.take_effects().events;
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, EventPayload::MemoryContentAppended { .. })),
        "a corroboration records no new content entry"
    );
    let attested = attestations(&events);
    assert_eq!(attested.len(), 1, "one attestation is buffered");
    assert_eq!(
        attested[0],
        (
            entry_id,
            Teller::Participant(speaker),
            Visibility::Public,
            None
        ),
        "identical text records the new teller at public with no distinct phrasing"
    );
}

#[tokio::test]
async fn append_dedup_records_distinct_phrasing_when_it_differs() {
    // The attester's own wording is preserved on the attestation only when it differs from the entry
    // text (kept for history and the console).
    let dave = MemoryId::generate();
    let entry_id = EntryId::generate();
    let (graph, vectors, embedder, embedding) = seeded_entry(
        dave,
        entry_id,
        "is a senior developer",
        Teller::Agent,
        Visibility::Public,
    )
    .await;

    let speaker = MemoryId::generate();
    let clock = ManualClock::new(Timestamp::from_millis(2_000));
    let mut block = block_with_retrieval(
        graph,
        clock,
        Teller::Participant(speaker),
        Authority::Platform,
        embedder,
        Box::new(vectors),
    );
    block
        .append_dedup(
            dave,
            "works as a senior engineer",
            AppendOptions {
                visibility: Some(VisibilityChoice::Public),
                ..AppendOptions::default()
            },
            Some(&embedding),
        )
        .unwrap();
    let events = block.take_effects().events;
    let attested = attestations(&events);
    assert_eq!(
        attested[0].3.as_deref(),
        Some("works as a senior engineer"),
        "differing wording is kept as the attestation's phrasing"
    );
}

#[tokio::test]
async fn append_dedup_same_teller_class_still_errors() {
    // The founding teller re-recording their own fact is the DuplicateEntry teachable error, not an
    // attestation to oneself.
    let dave = MemoryId::generate();
    let entry_id = EntryId::generate();
    let (graph, vectors, embedder, embedding) = seeded_entry(
        dave,
        entry_id,
        "is a senior developer",
        Teller::Agent,
        Visibility::Public,
    )
    .await;

    let clock = ManualClock::new(Timestamp::from_millis(2_000));
    let mut block = block_with_retrieval(
        graph,
        clock,
        Teller::Agent,
        Authority::Platform,
        embedder,
        Box::new(vectors),
    );
    let error = block
        .append_dedup(
            dave,
            "is a senior developer",
            AppendOptions::default(),
            Some(&embedding),
        )
        .unwrap_err();
    assert!(
        matches!(error, MemoryError::DuplicateEntry { .. }),
        "expected DuplicateEntry for the founding teller's own repeat, got {error:?}",
    );
}

#[tokio::test]
async fn append_dedup_private_confirmation_becomes_a_hidden_attestation() {
    // A participant confirming a public fact takes the write-time default (private to that teller), so
    // the corroboration lands as a hidden attestation on the still-public entry.
    let dave = MemoryId::generate();
    let entry_id = EntryId::generate();
    let (graph, vectors, embedder, embedding) = seeded_entry(
        dave,
        entry_id,
        "is a senior developer",
        Teller::Agent,
        Visibility::Public,
    )
    .await;

    let speaker = MemoryId::generate();
    let clock = ManualClock::new(Timestamp::from_millis(2_000));
    let mut block = block_with_retrieval(
        graph,
        clock,
        Teller::Participant(speaker),
        Authority::Platform,
        embedder,
        Box::new(vectors),
    );
    block
        .append_dedup(
            dave,
            "is a senior developer",
            AppendOptions::default(),
            Some(&embedding),
        )
        .unwrap();
    let events = block.take_effects().events;
    let attested = attestations(&events);
    assert_eq!(
        attested[0].2,
        Visibility::PrivateToTeller,
        "an unclassified participant confirmation is a private (hidden) attestation"
    );
}

#[tokio::test]
async fn append_dedup_idempotent_reattest_is_a_noop() {
    // A teller already attesting at the resolved posture is a success no-op — read-your-writes folds
    // the block's own pending attestation, so a second identical confirmation buffers nothing further.
    let dave = MemoryId::generate();
    let entry_id = EntryId::generate();
    let (graph, vectors, embedder, embedding) = seeded_entry(
        dave,
        entry_id,
        "is a senior developer",
        Teller::Agent,
        Visibility::Public,
    )
    .await;

    let speaker = MemoryId::generate();
    let clock = ManualClock::new(Timestamp::from_millis(2_000));
    let mut block = block_with_retrieval(
        graph,
        clock,
        Teller::Participant(speaker),
        Authority::Platform,
        embedder,
        Box::new(vectors),
    );
    let opts = || AppendOptions {
        visibility: Some(VisibilityChoice::Public),
        ..AppendOptions::default()
    };
    block
        .append_dedup(dave, "is a senior developer", opts(), Some(&embedding))
        .unwrap();
    let second = block
        .append_dedup(dave, "is a senior developer", opts(), Some(&embedding))
        .unwrap();
    let AppendOutcome::Corroborated(corroboration) = second else {
        panic!("expected a corroboration");
    };
    assert!(
        corroboration.note.contains("already attested"),
        "the second identical confirmation reports an already-attested no-op: {}",
        corroboration.note
    );
    let events = block.take_effects().events;
    assert_eq!(
        attestations(&events).len(),
        1,
        "only the first confirmation buffers an attestation"
    );
}

#[tokio::test]
async fn append_dedup_distinct_from_skips_the_named_entry() {
    // Naming the near-duplicate as distinct_from records the write as a genuinely separate entry
    // rather than folding it in as a corroboration.
    let dave = MemoryId::generate();
    let entry_id = EntryId::generate();
    let (graph, vectors, embedder, embedding) = seeded_entry(
        dave,
        entry_id,
        "is a senior developer",
        Teller::Agent,
        Visibility::Public,
    )
    .await;

    let speaker = MemoryId::generate();
    let clock = ManualClock::new(Timestamp::from_millis(2_000));
    let mut block = block_with_retrieval(
        graph,
        clock,
        Teller::Participant(speaker),
        Authority::Platform,
        embedder,
        Box::new(vectors),
    );
    let outcome = block
        .append_dedup(
            dave,
            "is a senior developer",
            AppendOptions {
                visibility: Some(VisibilityChoice::Public),
                distinct_from: Some(EntrySelector::Id(entry_id)),
                ..AppendOptions::default()
            },
            Some(&embedding),
        )
        .unwrap();
    let AppendOutcome::Appended { entry: new_id, .. } = outcome else {
        panic!("expected a fresh append when the sole hit is distinct_from");
    };
    assert_ne!(new_id, entry_id);
    let events = block.take_effects().events;
    assert!(
        events.iter().any(|event| matches!(
            event,
            EventPayload::MemoryContentAppended { entry_id: appended, .. } if *appended == new_id
        )),
        "a new content entry is recorded"
    );
    assert!(
        attestations(&events).is_empty(),
        "no corroboration is recorded when the hit is skipped"
    );
}
