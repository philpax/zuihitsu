//! Explicit attestation and the cross-class advisory: an attestation may not widen an entry's audience,
//! an explicit `told_by` attributes the endorsement, and an all-audience cross-class near-duplicate
//! rides back a teaching advisory while a cross-class confidence stays wholly invisible.

use std::sync::Arc;

use super::{
    AppendOptions, Authority, MemoryError, VisibilityChoice, attestations, block,
    block_with_retrieval,
};
use crate::{
    clock::ManualClock,
    event::{EventPayload, EventSource, Teller, Visibility},
    graph::Graph,
    ids::{EntryId, MemoryId, MemoryName, Namespace},
    memory::memory_block::{AppendOutcome, EntrySelector},
    model::embed::{CpuEmbedder, Embedder},
    store::{MemoryStore, Store},
    time::Timestamp,
    vector::{InMemoryVectorIndex, VectorIndex, VectorRecord},
};

/// A graph seeded with `person/dave` holding one live entry with an explicit teller and posture — the
/// fixture the explicit-attest tests resolve their target against.
fn graph_with_entry(
    dave: MemoryId,
    entry_id: EntryId,
    told_by: Teller,
    visibility: Visibility,
) -> Graph {
    let mut store = MemoryStore::new();
    store
        .append(
            Timestamp::from_millis(1_000),
            EventSource::Agent,
            vec![
                EventPayload::memory_created(dave, Namespace::Person.with_name("dave")),
                EventPayload::MemoryContentAppended {
                    id: dave,
                    entry_id,
                    asserted_at: Timestamp::from_millis(1_000),
                    occurred_at: None,
                    text: "is a senior developer".to_owned(),
                    told_by,
                    told_in: None,
                    visibility,
                },
            ],
        )
        .unwrap();
    let mut graph = Graph::open_in_memory().unwrap();
    graph.materialize_from(&store).unwrap();
    graph
}

#[test]
fn attest_rejects_a_wider_posture() {
    // The audience-widening invariant's real check: a public attestation on a private-founded entry is
    // refused — the fact is held at a narrower audience.
    let dave = MemoryId::generate();
    let entry_id = EntryId::generate();
    let confider = MemoryId::generate();
    let graph = graph_with_entry(
        dave,
        entry_id,
        Teller::Participant(confider),
        Visibility::PrivateToTeller,
    );
    let clock = ManualClock::new(Timestamp::from_millis(2_000));
    let mut block = block(graph, clock, Teller::Agent, Authority::Platform);
    let error = block
        .attest(
            dave,
            EntrySelector::Id(entry_id),
            AppendOptions {
                visibility: Some(VisibilityChoice::Public),
                ..AppendOptions::default()
            },
        )
        .unwrap_err();
    assert!(
        matches!(error, MemoryError::AttestationWiderThanEntry),
        "expected AttestationWiderThanEntry, got {error:?}",
    );
}

#[test]
fn attest_by_explicit_told_by_records_the_attestation() {
    // An explicit attest attributes the endorsement to opts.told_by, mirroring append.
    let dave = MemoryId::generate();
    let entry_id = EntryId::generate();
    let graph = graph_with_entry(dave, entry_id, Teller::Agent, Visibility::Public);
    let clock = ManualClock::new(Timestamp::from_millis(2_000));
    let mut block = block(graph, clock, Teller::Agent, Authority::Platform);

    let erin = MemoryId::generate();
    let corroboration = block
        .attest(
            dave,
            EntrySelector::Id(entry_id),
            AppendOptions {
                told_by: Some(Teller::Participant(erin)),
                visibility: Some(VisibilityChoice::Public),
                ..AppendOptions::default()
            },
        )
        .unwrap();
    assert_eq!(corroboration.entry, entry_id);
    let events = block.take_effects().events;
    let attested = attestations(&events);
    assert_eq!(
        attested,
        vec![(
            entry_id,
            Teller::Participant(erin),
            Visibility::Public,
            None
        )],
        "the attestation is attributed to the explicit told_by teller"
    );
}

#[tokio::test]
async fn a_cross_class_near_duplicate_surfaces_an_advisory_note() {
    // The same fact recorded about a different subject is never a capture — a different subject is
    // a different fact by policy — but an all-audience cross-class near-duplicate rides back as a
    // teaching advisory, steering the agent toward one record plus links instead of one re-phrasing
    // per participant. A cross-class confidence must stay wholly invisible: no advisory, no
    // snippet, no existence.
    let embedder: Arc<dyn Embedder> = CpuEmbedder::shared();

    let rowan_name: MemoryName = Namespace::Person.with_name("rowan").into();
    let rowan: MemoryId = MemoryId::generate();
    let erin: MemoryId = MemoryId::generate();
    let public_entry = EntryId::generate();
    let confided_entry = EntryId::generate();

    let mut store = MemoryStore::new();
    store
        .append(
            Timestamp::from_millis(1_000),
            EventSource::Agent,
            vec![
                EventPayload::memory_created(rowan, rowan_name.clone()),
                EventPayload::memory_created(erin, Namespace::Person.with_name("erin")),
                EventPayload::MemoryContentAppended {
                    id: rowan,
                    entry_id: public_entry,
                    asserted_at: Timestamp::from_millis(1_000),
                    occurred_at: None,
                    text: "built the grain harvester prototype in just two days after the workshop demo".to_owned(),
                    told_by: Teller::Agent,
                    told_in: None,
                    visibility: Visibility::Public,
                },
                EventPayload::MemoryContentAppended {
                    id: rowan,
                    entry_id: confided_entry,
                    asserted_at: Timestamp::from_millis(1_000),
                    occurred_at: None,
                    text: "is quietly planning to leave the collective and move to the coast before winter begins".to_owned(),
                    told_by: Teller::Participant(erin),
                    told_in: None,
                    visibility: Visibility::PrivateToTeller,
                },
            ],
        )
        .unwrap();
    let mut graph = Graph::open_in_memory().unwrap();
    graph.materialize_from(&store).unwrap();

    let mut vectors = InMemoryVectorIndex::new();
    for (entry_id, text) in [
        (
            public_entry,
            "built the grain harvester prototype in just two days after the workshop demo",
        ),
        (
            confided_entry,
            "is quietly planning to leave the collective and move to the coast before winter begins",
        ),
    ] {
        let contextual = crate::model::embed::contextual_text(rowan_name.as_str(), text);
        let embedding = embedder.embed(&[contextual]).await.unwrap().remove(0);
        vectors
            .upsert(VectorRecord {
                id: crate::model::index::VectorKey::EntryContextual(entry_id).to_vector_id(),
                embedding,
                model_id: embedder.model_id().into(),
            })
            .unwrap();
    }

    let clock = ManualClock::new(Timestamp::from_millis(2_000));
    let mut block = block_with_retrieval(
        graph,
        clock,
        Teller::Agent,
        Authority::Platform,
        embedder.clone(),
        Box::new(vectors),
    );
    // The same fact filed onto a different subject: the write proceeds, with the advisory naming
    // the all-audience original.
    let dana = block
        .create(Namespace::Person.with_name("dana"), None)
        .unwrap();
    let text = "built the grain harvester prototype in just two days after the workshop demo";
    let embedding = embedder
        .embed(&[crate::model::embed::contextual_text("person/dana", text)])
        .await
        .unwrap()
        .remove(0);
    let opts = AppendOptions {
        visibility: Some(VisibilityChoice::Public),
        ..AppendOptions::default()
    };
    let outcome = block
        .append_dedup(dana, text, opts, Some(&embedding))
        .unwrap();
    let AppendOutcome::Appended { advisory, .. } = outcome else {
        panic!("a cross-class near-duplicate must append, not capture: {outcome:?}");
    };
    let advisory = advisory.expect("the all-audience cross-class near-duplicate advises");
    assert!(advisory.contains("person/rowan"), "{advisory}");
    assert!(!advisory.contains("leave the collective"), "{advisory}");

    // A near-duplicate of the cross-class confidence stays wholly invisible.
    let text =
        "is quietly planning to leave the collective and move to the coast before winter begins";
    let embedding = embedder
        .embed(&[crate::model::embed::contextual_text("person/dana", text)])
        .await
        .unwrap()
        .remove(0);
    let opts = AppendOptions {
        visibility: Some(VisibilityChoice::Public),
        ..AppendOptions::default()
    };
    let outcome = block
        .append_dedup(dana, text, opts, Some(&embedding))
        .unwrap();
    let AppendOutcome::Appended { advisory, .. } = outcome else {
        panic!("a cross-class confidence must never capture: {outcome:?}");
    };
    assert!(
        advisory.is_none(),
        "a cross-class confidence must never surface an advisory: {advisory:?}"
    );
}
