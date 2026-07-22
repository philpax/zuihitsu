//! The authority, anchor, and merge write gates.

use super::{
    AppendOptions, Authority, MemoryError, VisibilityChoice, block, graph_with_self, told,
};
use crate::{
    clock::ManualClock,
    event::{Cardinality, EventPayload, EventSource, LinkSource, MergeProposalSource, Teller},
    graph::Graph,
    ids::{MemoryId, Namespace, NamespacedMemoryName},
    store::{MemoryStore, Store},
    time::Timestamp,
    vocabulary::RelationName,
};

#[test]
fn platform_authority_cannot_write_self() {
    let (graph, self_id) = graph_with_self();
    let clock = ManualClock::new(Timestamp::from_millis(2_000));
    let mut block = block(graph, clock, Teller::Agent, Authority::Platform);
    let other = block
        .create(Namespace::Person.with_name("marcus"), None)
        .unwrap();

    // Appending to self, and a link with self at either endpoint, are all barred.
    assert!(matches!(
        block
            .append(self_id, "I am sentient", AppendOptions::default())
            .unwrap_err(),
        MemoryError::SelfWriteForbidden
    ));
    assert!(matches!(
        block
            .link(self_id, other, RelationName::CreatedBy, None)
            .unwrap_err(),
        MemoryError::SelfWriteForbidden
    ));
    assert!(matches!(
        block
            .link(other, self_id, RelationName::CreatedBy, None)
            .unwrap_err(),
        MemoryError::SelfWriteForbidden
    ));
    assert!(matches!(
        block
            .unlink(self_id, other, RelationName::CreatedBy)
            .unwrap_err(),
        MemoryError::SelfWriteForbidden
    ));
}

#[test]
fn content_writes_to_the_operator_anchor_are_forbidden_but_links_are_not() {
    // A graph holding the person/operator anchor and the same_as relation.
    let mut store = MemoryStore::new();
    let operator_id = MemoryId::generate();
    store
        .append(
            Timestamp::from_millis(1_000),
            EventSource::Agent,
            vec![
                EventPayload::memory_created(operator_id, NamespacedMemoryName::operator()),
                EventPayload::LinkTypeRegistered {
                    name: RelationName::SameAs,
                    inverse: RelationName::SameAs,
                    from_card: Cardinality::Many,
                    to_card: Cardinality::Many,
                    symmetric: true,
                    reflexive: false,
                    description: String::new(),
                },
            ],
        )
        .unwrap();
    let mut graph = Graph::open_in_memory().unwrap();
    graph.materialize_from(&store).unwrap();

    let clock = ManualClock::new(Timestamp::from_millis(2_000));
    let mut block = block(graph, clock, Teller::Agent, Authority::Operator);
    let real = block
        .create(Namespace::Person.with_name("marcus"), None)
        .unwrap();

    // Recording content on the anchor is barred — even under operator authority.
    assert!(matches!(
        block
            .append(operator_id, "Real name is Marcus", AppendOptions::default())
            .unwrap_err(),
        MemoryError::OperatorWriteForbidden
    ));
    // The merge link to the anchor is not content, so it is allowed.
    assert!(
        block
            .link(operator_id, real, RelationName::SameAs, None)
            .is_ok()
    );
}

#[test]
fn operator_authority_may_write_self_and_links_carry_operator() {
    let (graph, self_id) = graph_with_self();
    let clock = ManualClock::new(Timestamp::from_millis(2_000));
    let mut block = block(graph, clock, Teller::Agent, Authority::Operator);
    let marcus = block
        .create(Namespace::Person.with_name("marcus"), None)
        .unwrap();

    // The same writes that platform authority bars all succeed from the console.
    block
        .append(
            self_id,
            "I exist to keep Marcus's memory.",
            AppendOptions::default(),
        )
        .unwrap();
    block
        .link(self_id, marcus, RelationName::CreatedBy, None)
        .unwrap();

    // The operator-authored link carries operator provenance, not the agent's own.
    let source = block
        .into_effects()
        .events
        .into_iter()
        .find_map(|event| match event {
            EventPayload::LinkCreated { source, .. } => Some(source),
            _ => None,
        })
        .unwrap();
    assert_eq!(source, LinkSource::Operator);
}

#[test]
fn platform_authority_same_as_link_routes_to_a_merge_proposal() {
    let (graph, _self_id) = graph_with_self();
    let clock = ManualClock::new(Timestamp::from_millis(2_000));
    let mut block = block(graph, clock, Teller::Agent, Authority::Platform);
    let dave = block
        .create(Namespace::Person.with_name("dave"), None)
        .unwrap();
    let dave_chat = block
        .create(Namespace::Person.with_name("dave@chat"), None)
        .unwrap();

    // A sibling append rides in the same block; it must survive the same_as handling.
    block
        .append(
            dave,
            "handles the deploys",
            AppendOptions {
                visibility: Some(VisibilityChoice::Public),
                ..AppendOptions::default()
            },
        )
        .unwrap();

    // The agent reading `link("same_as", …)` as an identity binding does not crash the block: the
    // create routes to the proposal path, buffering an inert `MergeProposed` (no `same_as`, no rollback).
    block
        .link(dave, dave_chat, RelationName::SameAs, None)
        .unwrap();

    // A retraction, by contrast, stays operator-only — the agent can neither assert nor undo a merge.
    assert!(matches!(
        block
            .unlink(dave, dave_chat, RelationName::SameAs)
            .unwrap_err(),
        MemoryError::MergeForbidden
    ));

    // The block commits a `MergeProposed` (agent-sourced, no rationale) rather than a `same_as`
    // `LinkCreated`, and the innocent sibling append survives alongside it.
    let events = block.into_effects().events;
    let proposed = events.iter().any(|event| {
        matches!(
            event,
            EventPayload::MergeProposed {
                from,
                to,
                source: MergeProposalSource::Agent,
                rationale: None,
            } if *from == dave && *to == dave_chat
        )
    });
    assert!(proposed, "the same_as link routes to a MergeProposed");
    let no_same_as = !events.iter().any(|event| {
        matches!(
            event,
            EventPayload::LinkCreated { relation, .. } if *relation == RelationName::SameAs
        )
    });
    assert!(no_same_as, "no same_as link is authored from a turn");
    let sibling_survived = events.iter().any(
        |event| matches!(event, EventPayload::MemoryContentAppended { id, .. } if *id == dave),
    );
    assert!(sibling_survived, "the sibling append is not rolled back");
}

#[test]
fn operator_authority_may_assert_a_same_as_merge() {
    let (graph, _self_id) = graph_with_self();
    let clock = ManualClock::new(Timestamp::from_millis(2_000));
    let mut block = block(graph, clock, Teller::Agent, Authority::Operator);
    let dave = block
        .create(Namespace::Person.with_name("dave"), None)
        .unwrap();
    let dave_chat = block
        .create(Namespace::Person.with_name("dave@chat"), None)
        .unwrap();

    block
        .link(dave, dave_chat, RelationName::SameAs, None)
        .unwrap();
}

#[test]
fn agent_authority_can_supersede_foreign_confidence() {
    // A maintenance pass (Agent authority) can supersede a confidence told by a different participant —
    // the foreign-confidence gate passes for non-Platform authority. Platform would be blocked.
    let graph = Graph::open_in_memory().unwrap();
    let clock = ManualClock::new(Timestamp::from_millis(1_000));
    let other = MemoryId::generate();
    let mut block = block(graph, clock, Teller::Agent, Authority::Agent);
    let topic = block
        .create(Namespace::Topic.with_name("aside"), None)
        .unwrap();
    let old = block
        .append(
            topic,
            "confided",
            told(Teller::Participant(other), VisibilityChoice::Private),
        )
        .unwrap();
    let new = block
        .append(
            topic,
            "consolidated",
            told(Teller::Agent, VisibilityChoice::Public),
        )
        .unwrap();
    // Under Platform authority this would return ForeignConfidenceSupersedeForbidden.
    block.supersede(topic, old, new).unwrap();
}

#[test]
fn agent_authority_cannot_write_self() {
    // The Agent authority has supersede and same_as powers, but self writes are still operator-only.
    let (graph, self_id) = graph_with_self();
    let clock = ManualClock::new(Timestamp::from_millis(2_000));
    let mut block = block(graph, clock, Teller::Agent, Authority::Agent);

    assert!(matches!(
        block
            .append(self_id, "I am sentient", AppendOptions::default())
            .unwrap_err(),
        MemoryError::SelfWriteForbidden
    ));
}

#[test]
fn agent_authority_can_assert_same_as() {
    // A maintenance pass asserts same_as directly — no merge proposal, no operator confirmation needed.
    // Platform authority would route to a MergeProposed; Agent authority asserts the link.
    let (graph, _self_id) = graph_with_self();
    let clock = ManualClock::new(Timestamp::from_millis(2_000));
    let mut block = block(graph, clock, Teller::Agent, Authority::Agent);
    let dave = block
        .create(Namespace::Person.with_name("dave"), None)
        .unwrap();
    let dave_canonical = block
        .create(Namespace::Person.with_name("dave-canonical"), None)
        .unwrap();

    block
        .link(dave, dave_canonical, RelationName::SameAs, None)
        .unwrap();

    // The link is a direct same_as assertion, not a MergeProposed.
    let events = block.into_effects().events;
    let has_same_as = events.iter().any(|event| {
        matches!(
            event,
            EventPayload::LinkCreated { relation, .. } if *relation == RelationName::SameAs
        )
    });
    assert!(has_same_as, "Agent authority asserts same_as directly");
    let no_proposal = !events
        .iter()
        .any(|event| matches!(event, EventPayload::MergeProposed { .. }));
    assert!(no_proposal, "no MergeProposed is buffered");
}
