//! The class-unified `memory.list` read (`list_by_prefix`): a `same_as` identity collapses to one row
//! under its class primary, so a person spanning a canonical profile and its platform stubs lists once
//! rather than as several near-identical rows. A lone memory lists as itself, the common path, and a
//! collapsed row falls back to a member's description when the primary's own is empty.

use crate::{
    clock::ManualClock,
    event::{Cardinality, EventPayload, EventSource, LinkPosture, LinkSource, Teller, Visibility},
    graph::Graph,
    ids::{MemoryId, Namespace},
    memory::memory_block::{
        MemoryBlock,
        tests::{Authority, block},
    },
    store::{MemoryStore, Store},
    time::Timestamp,
    vocabulary::RelationName,
};

/// Materialize a fresh in-memory graph from `events` — the committed state the read runs against.
fn graph_from(events: Vec<EventPayload>) -> Graph {
    let mut store = MemoryStore::new();
    store
        .append(Timestamp::from_millis(1_000), EventSource::Agent, events)
        .unwrap();
    let mut graph = Graph::open_in_memory().unwrap();
    graph.materialize_from(&store).unwrap();
    graph
}

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

/// A read-only block over `graph`, enough to drive `list_by_prefix` (a committed-only graph read).
fn read_block(graph: Graph) -> MemoryBlock {
    block(
        graph,
        ManualClock::new(Timestamp::from_millis(2_000)),
        Teller::Agent,
        Authority::Agent,
    )
}

#[tokio::test]
async fn a_merged_identity_lists_once_under_its_primary() {
    // A canonical profile (`person/rowan`) and a platform stub (`person/rowan@discord`), bound `same_as`
    // with the bare profile designated primary. The stem `person/` matches both members, but the list
    // collapses them to one row under the primary.
    let rowan = MemoryId::generate();
    let stub = MemoryId::generate();
    let graph = graph_from(vec![
        same_as_relation(),
        EventPayload::memory_created(rowan, Namespace::Person.with_name("rowan")),
        EventPayload::memory_description_regenerated(rowan, "Rowan, the designer.", None),
        EventPayload::memory_created(stub, Namespace::Person.with_name("rowan@discord")),
        EventPayload::memory_description_regenerated(stub, "rowan on the server.", None),
        same_as(rowan, stub),
        EventPayload::class_primary_designated(rowan, true),
    ]);
    let block = read_block(graph);

    let rows = block.list_by_prefix("person/").unwrap();
    assert_eq!(rows.len(), 1, "the two members collapse to one row");
    // The row's id is the bare canonical profile, not the platform stub — it renders under the primary.
    assert_eq!(rows[0].id, rowan, "the row renders under the class primary");
    assert_ne!(rows[0].id, stub, "not under the platform stub");
    // The primary carries its own description, so the row reads it lazily (no override).
    assert_eq!(rows[0].description, None);
}

#[tokio::test]
async fn a_collapsed_row_falls_back_to_a_member_description_when_the_primary_has_none() {
    // A freshly-minted canonical profile with no description yet, beside a described stub. The row must
    // list under the primary but borrow the stub's description, so it does not read as a blank line.
    let rowan = MemoryId::generate();
    let stub = MemoryId::generate();
    let graph = graph_from(vec![
        same_as_relation(),
        EventPayload::memory_created(rowan, Namespace::Person.with_name("rowan")),
        EventPayload::memory_created(stub, Namespace::Person.with_name("rowan@discord")),
        EventPayload::memory_description_regenerated(stub, "rowan on the server.", None),
        same_as(rowan, stub),
        EventPayload::class_primary_designated(rowan, true),
    ]);
    let block = read_block(graph);

    let rows = block.list_by_prefix("person/").unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, rowan, "the row still renders under the primary");
    assert_eq!(
        rows[0].description.as_deref(),
        Some("rowan on the server."),
        "the empty-primary row borrows the member's description"
    );
}

#[tokio::test]
async fn a_lone_memory_lists_as_itself_unchanged() {
    // A memory in no `same_as` class lists as itself with no description override — the common path.
    let erin = MemoryId::generate();
    let graph = graph_from(vec![
        EventPayload::memory_created(erin, Namespace::Person.with_name("erin")),
        EventPayload::memory_description_regenerated(erin, "Erin, on the ops team.", None),
    ]);
    let block = read_block(graph);

    let rows = block.list_by_prefix("person/erin").unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, erin);
    assert_eq!(
        rows[0].description, None,
        "a lone memory reads its own description lazily, no override"
    );
}
