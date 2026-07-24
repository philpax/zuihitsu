//! Class-aware link reads and the class-wide redundancy check: a relationship recorded against a raw
//! platform stub renders under the neighbour's canonical primary (collapsing parallel edges is each
//! caller's, after its visibility filter), and [`Graph::link_between`] recognises an edge already stored
//! against another member of the two identities. Exercised against materialized state, since the primary
//! is derived by the class recompute the materializer runs on every `same_as` change.

use crate::{
    event::{Cardinality, EventPayload, LinkPosture, LinkSource, Visibility},
    graph::tests::{materialized, mentor_relation},
    ids::{MemoryId, MemoryName, Namespace},
    vocabulary::RelationName,
};

/// The symmetric `same_as` relation registration a merge needs.
fn same_as_relation() -> EventPayload {
    EventPayload::LinkTypeRegistered {
        name: RelationName::SameAs,
        inverse: RelationName::SameAs,
        from_card: Cardinality::Many,
        to_card: Cardinality::Many,
        symmetric: true,
        reflexive: false,
        description: String::new(),
    }
}

/// An operator-asserted `same_as` binding two stubs (no teller, public).
fn same_as(a: MemoryId, b: MemoryId) -> EventPayload {
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
    )
}

/// A public `mentor_of` edge with agent provenance and no teller.
fn mentors(a: MemoryId, b: MemoryId) -> EventPayload {
    EventPayload::link_created(
        a,
        b,
        RelationName::new("mentor_of"),
        LinkPosture {
            source: LinkSource::Agent,
            told_by: None,
            told_in: None,
            visibility: Visibility::Public,
        },
    )
}

#[test]
fn class_neighbor_links_render_the_far_primary() {
    // A queried identity (rowan + a platform stub, rowan the designated primary) mentors a far identity
    // (erin + a platform stub, erin the designated primary). The relationship is recorded twice — once
    // between the two stubs, once between the two primaries — so the raw graph holds two parallel edges
    // reaching the same far identity through different raw endpoints.
    let rowan = MemoryId::generate();
    let rowan_stub = MemoryId::generate();
    let erin = MemoryId::generate();
    let erin_stub = MemoryId::generate();
    let (_store, graph) = materialized(vec![
        same_as_relation(),
        mentor_relation(),
        EventPayload::memory_created(rowan, Namespace::Person.with_name("rowan")),
        EventPayload::memory_created(rowan_stub, Namespace::Person.with_name("9001@testplat")),
        EventPayload::memory_created(erin, Namespace::Person.with_name("erin")),
        EventPayload::memory_created(erin_stub, Namespace::Person.with_name("9002@testplat")),
        same_as(rowan, rowan_stub),
        same_as(erin, erin_stub),
        EventPayload::class_primary_designated(rowan, true),
        EventPayload::class_primary_designated(erin, true),
        // The relationship is attached to the stubs and, redundantly, to the primaries.
        mentors(rowan_stub, erin_stub),
        mentors(rowan, erin),
    ]);

    // Both parallel edges are returned — deduplication is the caller's, after its visibility filter —
    // but each renders under the far primary's canonical handle, and the set is queryable from either
    // member of the queried class.
    for member in [rowan, rowan_stub] {
        let neighbours = graph.class_neighbor_links(member).unwrap();
        assert_eq!(
            neighbours.len(),
            2,
            "parallel edges stay distinct — each carries its own visibility for the caller to weigh",
        );
        for neighbour in &neighbours {
            assert_eq!(neighbour.other, erin, "resolved to the far primary");
            assert_eq!(
                neighbour.other_name,
                MemoryName::new("person/erin"),
                "rendered under the far primary's canonical name, not the stub snowflake",
            );
            assert!(!neighbour.incoming, "the queried identity is the source");
        }
    }
}

#[test]
fn class_neighbor_links_on_a_lone_memory_are_unchanged() {
    // The common path: two lone memories, one edge between them. No class is involved, so the far
    // endpoint renders as itself and nothing is collapsed.
    let dave = MemoryId::generate();
    let erin = MemoryId::generate();
    let (_store, graph) = materialized(vec![
        mentor_relation(),
        EventPayload::memory_created(dave, Namespace::Person.with_name("dave")),
        EventPayload::memory_created(erin, Namespace::Person.with_name("erin")),
        mentors(dave, erin),
    ]);

    let neighbours = graph.class_neighbor_links(dave).unwrap();
    assert_eq!(neighbours.len(), 1);
    assert_eq!(neighbours[0].other, erin);
    assert_eq!(neighbours[0].other_name, MemoryName::new("person/erin"));
    assert!(!neighbours[0].incoming);
}

#[test]
fn link_between_matches_a_class_equivalent_edge() {
    // The relationship is stored only between the two stubs. Asked for against the two primaries — which
    // hold no edge of their own — `link_between` still finds it via the class-wide lookup, so a re-link
    // against the primaries is weighed against the existing stub edge rather than minting a parallel one.
    let rowan = MemoryId::generate();
    let rowan_stub = MemoryId::generate();
    let erin = MemoryId::generate();
    let erin_stub = MemoryId::generate();
    let mentor_of = RelationName::new("mentor_of");
    let (_store, graph) = materialized(vec![
        same_as_relation(),
        mentor_relation(),
        EventPayload::memory_created(rowan, Namespace::Person.with_name("rowan")),
        EventPayload::memory_created(rowan_stub, Namespace::Person.with_name("9001@testplat")),
        EventPayload::memory_created(erin, Namespace::Person.with_name("erin")),
        EventPayload::memory_created(erin_stub, Namespace::Person.with_name("9002@testplat")),
        same_as(rowan, rowan_stub),
        same_as(erin, erin_stub),
        EventPayload::class_primary_designated(rowan, true),
        EventPayload::class_primary_designated(erin, true),
        mentors(rowan_stub, erin_stub),
    ]);

    // No exact edge between the primaries, yet the class-equivalent stub edge is found and its posture
    // returned.
    let posture = graph.link_between(rowan, erin, &mentor_of).unwrap();
    assert_eq!(
        posture,
        Some(LinkPosture {
            source: LinkSource::Agent,
            told_by: None,
            told_in: None,
            visibility: Visibility::Public,
        }),
        "the class-equivalent stub edge's posture is returned for the primary pair",
    );
    // It is also found in the canonical inverse direction (either label resolves to the one edge).
    assert!(
        graph
            .link_between(erin, rowan, &RelationName::new("mentored_by"))
            .unwrap()
            .is_some(),
    );
}

#[test]
fn link_between_on_a_lone_pair_is_exact() {
    // No class in play: `link_between` finds only the exact edge, and an unrelated pair finds nothing —
    // the common-path behaviour is unchanged.
    let dave = MemoryId::generate();
    let erin = MemoryId::generate();
    let frank = MemoryId::generate();
    let mentor_of = RelationName::new("mentor_of");
    let (_store, graph) = materialized(vec![
        mentor_relation(),
        EventPayload::memory_created(dave, Namespace::Person.with_name("dave")),
        EventPayload::memory_created(erin, Namespace::Person.with_name("erin")),
        EventPayload::memory_created(frank, Namespace::Person.with_name("frank")),
        mentors(dave, erin),
    ]);

    assert!(
        graph
            .link_between(dave, erin, &mentor_of)
            .unwrap()
            .is_some()
    );
    assert!(
        graph
            .link_between(dave, frank, &mentor_of)
            .unwrap()
            .is_none(),
        "an unrelated pair with no class holds no edge",
    );
}
