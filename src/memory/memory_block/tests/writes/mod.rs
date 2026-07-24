//! The write-path basics and teachable write errors, grouped by concern: create, rename, and link
//! basics; visibility defaults; the exclude opt; the class-spanning primary redirect; retraction;
//! deterministic replay and rollback; the dedup check and its auto-attest capture matrix; and explicit
//! attestation with the cross-class advisory. The fixtures shared across those groups live one level up
//! in the tests module; the helpers a couple of groups share are hoisted here.

mod attest;
mod basics;
mod dedup_attest;
mod dedup_rejection;
mod exclude;
mod redirect;
mod replay;
mod retract;
mod visibility;

pub(super) use super::{
    AppendOptions, Authority, MemoryError, VisibilityChoice, block, block_with_retrieval,
    graph_with_merged_pair,
};
use crate::{
    event::{Cardinality, EventPayload, EventSource, LinkPosture, LinkSource, Teller, Visibility},
    graph::Graph,
    ids::{EntryId, MemoryId, Namespace},
    store::{MemoryStore, Store},
    time::Timestamp,
    vocabulary::RelationName,
};

/// The seed events for a `same_as` class of two clean, platform-agnostic person handles with the
/// *later*-ULID stub pinned as the class primary by a `ClassPrimaryDesignated` — so the earliest-ULID
/// clean handle resolves to a *non-primary* member, the shape the class-spanning write redirect turns
/// on. Returns the seed events, the earliest-ULID stub (`person/dave`), and the designated primary
/// (`person/marcus`). The clean handles are bound to sorted ULIDs so the designation, not ULID order,
/// decides the primary regardless of the random low bits minted within one millisecond.
pub(super) fn designated_primary_seed() -> (Vec<EventPayload>, MemoryId, MemoryId) {
    let mut ids = [MemoryId::generate(), MemoryId::generate()];
    ids.sort();
    let [earliest, later] = ids;
    let events = vec![
        EventPayload::LinkTypeRegistered {
            name: RelationName::SameAs,
            inverse: RelationName::SameAs,
            from_card: Cardinality::Many,
            to_card: Cardinality::Many,
            symmetric: true,
            reflexive: false,
            description: String::new(),
        },
        EventPayload::memory_created(earliest, Namespace::Person.with_name("dave")),
        EventPayload::memory_created(later, Namespace::Person.with_name("marcus")),
        EventPayload::link_created(
            earliest,
            later,
            RelationName::SameAs,
            LinkPosture {
                source: LinkSource::Operator,
                told_by: None,
                told_in: None,
                visibility: Visibility::Public,
            },
        ),
        EventPayload::class_primary_designated(later, true),
    ];
    (events, earliest, later)
}

/// Materialize a set of seed events into a fresh in-memory graph — the committed state a block resolves
/// its write targets against.
pub(super) fn graph_from(events: Vec<EventPayload>) -> Graph {
    let mut store = MemoryStore::new();
    store
        .append(Timestamp::from_millis(1_000), EventSource::Agent, events)
        .unwrap();
    let mut graph = Graph::open_in_memory().unwrap();
    graph.materialize_from(&store).unwrap();
    graph
}

/// The buffered `EntryAttested` events of a block's effects, reduced to the fields the capture-matrix
/// tests assert over.
pub(super) fn attestations(
    events: &[EventPayload],
) -> Vec<(EntryId, Teller, Visibility, Option<String>)> {
    events
        .iter()
        .filter_map(|event| match event {
            EventPayload::EntryAttested {
                entry,
                teller,
                posture,
                phrasing,
                ..
            } => Some((*entry, teller.clone(), posture.clone(), phrasing.clone())),
            _ => None,
        })
        .collect()
}
