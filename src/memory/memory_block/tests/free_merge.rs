//! The agent free-merge rule: an `Authority::Agent` `same_as` asserts directly only when it binds a
//! freshly-minted empty profile, and otherwise routes to the merge-proposal machinery.

use crate::{
    clock::ManualClock,
    event::{Cardinality, EventPayload, EventSource, Teller, Visibility},
    graph::Graph,
    ids::{EntryId, MemoryId, Namespace},
    memory::memory_block::tests::{Authority, block},
    store::{MemoryStore, Store},
    time::Timestamp,
    vocabulary::RelationName,
};

/// A graph with the `same_as` relation and the given person memories, each optionally seeded with one
/// public content entry (so `is_empty_profile` sees a live entry for a populated one). Returns the
/// materialized graph.
fn graph_with(memories: &[(MemoryId, &str, bool)]) -> Graph {
    let mut events = vec![EventPayload::LinkTypeRegistered {
        name: RelationName::SameAs,
        inverse: RelationName::SameAs,
        from_card: Cardinality::Many,
        to_card: Cardinality::Many,
        symmetric: true,
        reflexive: false,
        description: String::new(),
    }];
    for (id, name, populated) in memories {
        events.push(EventPayload::memory_created(
            *id,
            Namespace::Person.with_name(*name),
        ));
        if *populated {
            events.push(EventPayload::MemoryContentAppended {
                id: *id,
                entry_id: EntryId::generate(),
                asserted_at: Timestamp::from_millis(1_000),
                occurred_at: None,
                text: "a recorded fact".to_owned(),
                told_by: Teller::Agent,
                told_in: None,
                visibility: Visibility::Public,
            });
        }
    }
    let mut store = MemoryStore::new();
    store
        .append(Timestamp::from_millis(1_000), EventSource::Agent, events)
        .unwrap();
    let mut graph = Graph::open_in_memory().unwrap();
    graph.materialize_from(&store).unwrap();
    graph
}

#[test]
fn binding_an_empty_profile_asserts_the_same_as_directly() {
    // The canonical-profile pass's case: a populated platform stub bound to a freshly-minted empty
    // profile. No visibility collapses (the profile carries nothing), so the `same_as` asserts
    // directly rather than routing to a merge proposal.
    let stub = MemoryId::generate();
    let profile = MemoryId::generate();
    let graph = graph_with(&[(stub, "dave@discord", true), (profile, "dave", false)]);
    let clock = ManualClock::new(Timestamp::from_millis(2_000));
    let mut block = block(graph, clock, Teller::Agent, Authority::Agent);

    block
        .link(
            stub,
            profile,
            RelationName::SameAs,
            Some(crate::memory::memory_block::LinkOptions {
                visibility: Some(crate::memory::memory_block::VisibilityChoice::Public),
                exclude: None,
            }),
        )
        .unwrap();

    let events = block.into_effects().events;
    assert!(
        events.iter().any(|event| matches!(
            event,
            EventPayload::LinkCreated { relation, .. } if *relation == RelationName::SameAs
        )),
        "an empty-profile bind asserts the same_as directly"
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, EventPayload::MergeProposed { .. })),
        "no merge is proposed when a side is empty"
    );
}

#[test]
fn binding_two_populated_profiles_proposes_a_merge() {
    // Both sides carry live entries, so an agent `same_as` would collapse two populated visibility
    // classes. It routes to the inert merge-proposal machinery instead — nothing merges until the
    // operator confirms.
    let a = MemoryId::generate();
    let b = MemoryId::generate();
    let graph = graph_with(&[(a, "dave@discord", true), (b, "dave@slack", true)]);
    let clock = ManualClock::new(Timestamp::from_millis(2_000));
    let mut block = block(graph, clock, Teller::Agent, Authority::Agent);

    block.link(a, b, RelationName::SameAs, None).unwrap();

    let events = block.into_effects().events;
    assert!(
        events
            .iter()
            .any(|event| matches!(event, EventPayload::MergeProposed { .. })),
        "a bind of two populated profiles proposes a merge"
    );
    assert!(
        !events.iter().any(|event| matches!(
            event,
            EventPayload::LinkCreated { relation, .. } if *relation == RelationName::SameAs
        )),
        "no same_as is asserted directly between two populated profiles"
    );
}

#[test]
fn a_member_entry_is_retractable_when_addressed_via_the_platform_stub() {
    // The maintenance passes iterate per memory and may address a class entry through the
    // platform-qualified stub rather than the bare member the entry lives on. The write target
    // stays the stub (connector-owned handles never redirect), but the liveness read spans the
    // whole `same_as` class, so the retraction must land.
    let bare = MemoryId::generate();
    let stub = MemoryId::generate();
    let entry = EntryId::generate();
    let mut events = vec![EventPayload::LinkTypeRegistered {
        name: RelationName::SameAs,
        inverse: RelationName::SameAs,
        from_card: Cardinality::Many,
        to_card: Cardinality::Many,
        symmetric: true,
        reflexive: false,
        description: String::new(),
    }];
    events.push(EventPayload::memory_created(
        bare,
        Namespace::Person.with_name("rowan"),
    ));
    events.push(EventPayload::memory_created(
        stub,
        Namespace::Person.with_name("1234567890@testchat"),
    ));
    events.push(EventPayload::MemoryContentAppended {
        id: bare,
        entry_id: entry,
        asserted_at: Timestamp::from_millis(1_000),
        occurred_at: None,
        text: "bot belonging to rowan".to_owned(),
        told_by: Teller::Agent,
        told_in: None,
        visibility: Visibility::Public,
    });
    events.push(EventPayload::link_created(
        stub,
        bare,
        RelationName::SameAs,
        crate::event::LinkPosture {
            source: crate::event::LinkSource::Operator,
            told_by: None,
            told_in: None,
            visibility: Visibility::Public,
        },
    ));
    let mut store = MemoryStore::new();
    store
        .append(Timestamp::from_millis(1_000), EventSource::Agent, events)
        .unwrap();
    let mut graph = Graph::open_in_memory().unwrap();
    graph.materialize_from(&store).unwrap();

    let clock = ManualClock::new(Timestamp::from_millis(2_000));
    let mut b = block(graph, clock, Teller::Agent, Authority::Agent);
    b.retract(stub, entry, "captured by the operated_by link", None)
        .expect("a class member's entry must be retractable via the stub");
}
