//! Operator-designated `same_as` class primaries: a designation pins the stub a class resolves
//! through, overriding the earliest-ULID default. Exercised against materialized state, since the
//! primary is derived by the class recompute the materializer runs on every `same_as` change.

use crate::{
    event::{Cardinality, EventPayload, EventSource, LinkPosture, LinkSource, Visibility},
    graph::tests::materialized,
    ids::{MemoryId, Namespace},
    store::{MemoryStore, Store},
    time::Timestamp,
    vocabulary::RelationName,
};

/// The symmetric `same_as` relation registration every merge test needs.
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

/// A `same_as` link between two stubs, operator-asserted (no teller, public).
fn merge(a: MemoryId, b: MemoryId) -> EventPayload {
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

/// Three person stubs, sorted by id so `lo < mid < hi` — the earliest-ULID rule would pick `lo`, so a
/// designation of `mid` or `hi` is the property under test.
fn sorted_stubs() -> (MemoryId, MemoryId, MemoryId) {
    let mut ids = [
        MemoryId::generate(),
        MemoryId::generate(),
        MemoryId::generate(),
    ];
    ids.sort();
    (ids[0], ids[1], ids[2])
}

#[test]
fn a_designation_wins_over_the_earliest_ulid() {
    let (lo, mid, hi) = sorted_stubs();
    let (_store, graph) = materialized(vec![
        same_as_relation(),
        EventPayload::memory_created(lo, Namespace::Person.with_name("pat")),
        EventPayload::memory_created(mid, Namespace::Person.with_name("patricia")),
        EventPayload::memory_created(hi, Namespace::Person.with_name("patty")),
        merge(lo, mid),
        merge(mid, hi),
        // The operator pins the latest-minted stub — the one earliest-ULID would never choose.
        EventPayload::class_primary_designated(hi, true),
    ]);

    // The whole class resolves through the designated stub, not `lo`.
    for member in [lo, mid, hi] {
        assert_eq!(graph.class_id(member).unwrap().unwrap(), hi);
    }
    assert!(graph.is_primary_designated(hi).unwrap());
    assert!(!graph.is_primary_designated(lo).unwrap());
}

#[test]
fn releasing_a_designation_falls_back_to_the_earliest_ulid() {
    let (lo, mid, hi) = sorted_stubs();
    let (_store, graph) = materialized(vec![
        same_as_relation(),
        EventPayload::memory_created(lo, Namespace::Person.with_name("pat")),
        EventPayload::memory_created(mid, Namespace::Person.with_name("patricia")),
        EventPayload::memory_created(hi, Namespace::Person.with_name("patty")),
        merge(lo, mid),
        merge(mid, hi),
        EventPayload::class_primary_designated(hi, true),
        // The operator changes their mind and releases the pin.
        EventPayload::class_primary_designated(hi, false),
    ]);

    for member in [lo, mid, hi] {
        assert_eq!(graph.class_id(member).unwrap().unwrap(), lo);
    }
    assert!(!graph.is_primary_designated(hi).unwrap());
}

#[test]
fn the_primary_is_independent_of_the_merge_order() {
    let (lo, mid, hi) = sorted_stubs();
    let creates = vec![
        same_as_relation(),
        EventPayload::memory_created(lo, Namespace::Person.with_name("pat")),
        EventPayload::memory_created(mid, Namespace::Person.with_name("patricia")),
        EventPayload::memory_created(hi, Namespace::Person.with_name("patty")),
    ];

    // Same class, same designation, but the merges arrive — and the designation lands — in different
    // orders. The primary must be identical, since the recompute examines the whole component.
    let mut forward = creates.clone();
    forward.extend([
        merge(lo, mid),
        merge(mid, hi),
        EventPayload::class_primary_designated(mid, true),
    ]);
    let mut reverse = creates;
    reverse.extend([
        EventPayload::class_primary_designated(mid, true),
        merge(hi, mid),
        merge(mid, lo),
    ]);

    let (_s1, forward_graph) = materialized(forward);
    let (_s2, reverse_graph) = materialized(reverse);
    assert_eq!(forward_graph.class_id(lo).unwrap().unwrap(), mid);
    assert_eq!(reverse_graph.class_id(lo).unwrap().unwrap(), mid);
}

#[test]
fn two_designations_fall_back_to_the_earliest_designated_stub() {
    let (lo, mid, hi) = sorted_stubs();
    let (_store, graph) = materialized(vec![
        same_as_relation(),
        EventPayload::memory_created(lo, Namespace::Person.with_name("pat")),
        EventPayload::memory_created(mid, Namespace::Person.with_name("patricia")),
        EventPayload::memory_created(hi, Namespace::Person.with_name("patty")),
        merge(lo, mid),
        merge(mid, hi),
        // Both `mid` and `hi` are pinned; the earliest-ULID designated stub (`mid`) wins, and `lo`
        // — earliest overall but undesignated — does not.
        EventPayload::class_primary_designated(hi, true),
        EventPayload::class_primary_designated(mid, true),
    ]);

    for member in [lo, mid, hi] {
        assert_eq!(graph.class_id(member).unwrap().unwrap(), mid);
    }
}

#[test]
fn a_designation_of_a_non_member_leaves_the_class_unchanged() {
    let (lo, mid, hi) = sorted_stubs();
    let (_store, graph) = materialized(vec![
        same_as_relation(),
        EventPayload::memory_created(lo, Namespace::Person.with_name("marcus@direct")),
        EventPayload::memory_created(mid, Namespace::Person.with_name("marcus@chat")),
        // `hi` is a separate person, never merged into the Marcus class.
        EventPayload::memory_created(hi, Namespace::Person.with_name("dave")),
        merge(lo, mid),
        // Designating the outsider has no bearing on the Marcus class's primary.
        EventPayload::class_primary_designated(hi, true),
    ]);

    assert_eq!(graph.class_id(lo).unwrap().unwrap(), lo);
    assert_eq!(graph.class_id(mid).unwrap().unwrap(), lo);
    // The outsider is its own singleton class, and its designation only pins itself.
    assert_eq!(graph.class_id(hi).unwrap().unwrap(), hi);
}

#[test]
fn unmerge_carries_the_designation_into_the_split_class() {
    let (lo, mid, hi) = sorted_stubs();
    // A three-member class merged as a chain lo–mid–hi, with `hi` designated primary. Retracting the
    // mid–hi edge splits `hi` off into its own class; the pin travels with it, and the remaining
    // {lo, mid} falls back to earliest-ULID.
    let (_store, graph) = materialized(vec![
        same_as_relation(),
        EventPayload::memory_created(lo, Namespace::Person.with_name("pat")),
        EventPayload::memory_created(mid, Namespace::Person.with_name("patricia")),
        EventPayload::memory_created(hi, Namespace::Person.with_name("patty")),
        merge(lo, mid),
        merge(mid, hi),
        EventPayload::class_primary_designated(hi, true),
        EventPayload::link_removed(mid, hi, RelationName::SameAs),
    ]);

    // `hi` split off, still carrying (and now trivially winning) its pin.
    assert_eq!(graph.class_id(hi).unwrap().unwrap(), hi);
    assert!(graph.is_primary_designated(hi).unwrap());
    // The remainder resolves through its earliest-ULID member, the pin having departed with `hi`.
    assert_eq!(graph.class_id(lo).unwrap().unwrap(), lo);
    assert_eq!(graph.class_id(mid).unwrap().unwrap(), lo);
}

#[test]
fn a_designated_class_refolds_identically_from_the_log() {
    let (lo, mid, hi) = sorted_stubs();
    let mut store = MemoryStore::new();
    store
        .append(
            Timestamp::from_millis(1_000),
            EventSource::Agent,
            vec![
                same_as_relation(),
                EventPayload::memory_created(lo, Namespace::Person.with_name("pat")),
                EventPayload::memory_created(mid, Namespace::Person.with_name("patricia")),
                EventPayload::memory_created(hi, Namespace::Person.with_name("patty")),
                merge(lo, mid),
                merge(mid, hi),
                EventPayload::class_primary_designated(hi, true),
            ],
        )
        .unwrap();

    // A fresh graph folded from the same log lands on the identical primary — the designation is a
    // pure function of the event stream, not of any in-memory recompute order.
    let mut first = crate::graph::Graph::open_in_memory().unwrap();
    first.materialize_from(&store).unwrap();
    let mut second = crate::graph::Graph::open_in_memory().unwrap();
    second.materialize_from(&store).unwrap();
    assert_eq!(first.class_id(lo).unwrap(), second.class_id(lo).unwrap());
    assert_eq!(first.class_id(lo).unwrap().unwrap(), hi);
}
