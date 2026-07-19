//! Shared fixtures for the ambient recall pass's tests, and the concern-grouped test submodules.
//!
//! The helpers here build an in-memory graph materialized from event payloads — the pattern the
//! graph's own search tests use, so the FTS index the pass reads is populated exactly as production's
//! is. Each submodule tests one concern: query extraction, URL extraction, hint rendering, the recall
//! orchestration, turn-token leads, and memory-reference leads.

mod extraction;
mod mems;
mod recall;
mod render;
mod tokens;
mod urls;

use crate::{
    event::{Cardinality, EventPayload, LinkPosture, LinkSource, Teller, Visibility},
    graph::Graph,
    ids::{EntryId, MemoryId, MemoryName, Namespace},
    store::{MemoryStore, Store},
    time::Timestamp,
    vocabulary::RelationName,
};

/// Build an in-memory graph materialized from `payloads` — the pattern the graph's own search
/// tests use, so the FTS index the pass reads is populated exactly as production's is.
fn materialized(payloads: Vec<EventPayload>) -> Graph {
    let mut store = MemoryStore::new();
    store
        .append(
            Timestamp::from_millis(1),
            crate::event::EventSource::Agent,
            payloads,
        )
        .unwrap();
    let mut graph = Graph::open_in_memory().unwrap();
    graph.materialize_from(&store).unwrap();
    graph
}

fn topic(id: MemoryId, name: &str, text: &str) -> Vec<EventPayload> {
    vec![
        EventPayload::memory_created(id, Namespace::Topic.with_name(name)),
        EventPayload::MemoryContentAppended {
            id,
            entry_id: EntryId::generate(),
            asserted_at: Timestamp::from_millis(1),
            occurred_at: None,
            text: text.to_owned(),
            told_by: Teller::Agent,
            told_in: None,
            visibility: Visibility::Public,
        },
    ]
}

/// A person stub named by its full handle, with one public content entry — the shape a merged
/// `same_as` class is built from.
fn person(id: MemoryId, name: &str, text: &str) -> Vec<EventPayload> {
    vec![
        EventPayload::memory_created(id, MemoryName::new(name)),
        EventPayload::MemoryContentAppended {
            id,
            entry_id: EntryId::generate(),
            asserted_at: Timestamp::from_millis(1),
            occurred_at: None,
            text: text.to_owned(),
            told_by: Teller::Agent,
            told_in: None,
            visibility: Visibility::Public,
        },
    ]
}

/// Merge `a` and `b` into one `same_as` class (operator-adjudicated), mirroring the graph merge
/// tests' payload pattern.
fn same_as(a: MemoryId, b: MemoryId) -> Vec<EventPayload> {
    vec![
        EventPayload::LinkTypeRegistered {
            name: RelationName::SameAs,
            inverse: RelationName::SameAs,
            from_card: Cardinality::Many,
            to_card: Cardinality::Many,
            symmetric: true,
            reflexive: false,
            description: String::new(),
        },
        EventPayload::link_created(
            a,
            b,
            RelationName::SameAs,
            LinkPosture {
                source: LinkSource::Operator,
                told_by: None,
                told_in: None,
                visibility: Visibility::Public,
            },
        ),
    ]
}

/// A dozen unrelated memories, so the FTS index carries a realistic corpus. bm25 collapses toward
/// zero on a one- or two-document index (every term is in every document, so its inverse-document
/// weight vanishes); the filler restores the score separation between a distinctive match and
/// common-word noise that a real instance sees.
fn filler() -> Vec<EventPayload> {
    (0..12)
        .flat_map(|i| {
            topic(
                MemoryId::generate(),
                &format!("filler{i}"),
                &format!("Unrelated note {i} about weather, lunch, and travel plans."),
            )
        })
        .collect()
}

/// Materialize `target` memories alongside the filler corpus.
fn corpus(target: Vec<EventPayload>) -> Graph {
    let mut payloads = target;
    payloads.extend(filler());
    materialized(payloads)
}

/// Two stubs of one identity, merged into one `same_as` class, both matching the kelp survey text.
fn merged_rowan() -> (Graph, MemoryId, MemoryId) {
    let direct = MemoryId::generate();
    let chat = MemoryId::generate();
    let mut payloads = person(
        direct,
        "person/rowan@direct",
        "Kelp survey planning at the harbour.",
    );
    payloads.extend(person(
        chat,
        "person/rowan@chat",
        "Kelp logistics notes from the night shift.",
    ));
    payloads.extend(same_as(direct, chat));
    (corpus(payloads), direct, chat)
}
