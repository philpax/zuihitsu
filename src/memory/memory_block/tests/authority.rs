//! The authority, anchor, and merge write gates.

use crate::{
    clock::ManualClock,
    event::{Cardinality, EventPayload, EventSource, LinkSource, MergeProposalSource, Teller},
    graph::Graph,
    ids::{MemoryId, MemoryName, Namespace, NamespacedMemoryName},
    memory::memory_block::tests::{
        AppendOptions, Authority, MemoryError, VisibilityChoice, block, graph_with_self, told,
    },
    store::{MemoryStore, Store},
    time::Timestamp,
    vocabulary::RelationName,
};

#[test]
fn platform_authority_cannot_write_self_content() {
    let (graph, self_id) = graph_with_self();
    let clock = ManualClock::new(Timestamp::from_millis(2_000));
    let mut block = block(graph, clock, Teller::Agent, Authority::Platform);

    // Appending to self is barred outside the console — the self model is content, operator-only.
    // Links touching self are not gated here; they are ordinary relationships (see the dedicated
    // self-link test), so the self-write guard covers content writes alone.
    assert!(matches!(
        block
            .append(self_id, "I am sentient", AppendOptions::default())
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

/// The seed for the self-link tests: `self`, a bare `person/rowan` (no entries of its own, so it reads
/// as an empty profile), and the `knows` and `same_as` relations. Returns the events and the two ids
/// so a test can materialize the committed state and fold a block's writes back over it.
fn self_and_person_events() -> (Vec<EventPayload>, MemoryId, MemoryId) {
    let self_id = MemoryId::generate();
    let rowan = MemoryId::generate();
    let events = vec![
        EventPayload::memory_created(self_id, MemoryName::new(MemoryName::SELF)),
        EventPayload::memory_created(rowan, Namespace::Person.with_name("rowan")),
        EventPayload::LinkTypeRegistered {
            name: RelationName::Knows,
            inverse: RelationName::KnownBy,
            from_card: Cardinality::Many,
            to_card: Cardinality::Many,
            symmetric: false,
            reflexive: false,
            description: String::new(),
        },
        EventPayload::LinkTypeRegistered {
            name: RelationName::SameAs,
            inverse: RelationName::SameAs,
            from_card: Cardinality::Many,
            to_card: Cardinality::Many,
            symmetric: true,
            reflexive: false,
            description: String::new(),
        },
    ];
    (events, self_id, rowan)
}

/// Materialize a fresh in-memory graph from `events` — the committed state a block reads against.
fn materialize(events: &[EventPayload]) -> Graph {
    let mut store = MemoryStore::new();
    store
        .append(
            Timestamp::from_millis(1_000),
            EventSource::Agent,
            events.to_vec(),
        )
        .unwrap();
    let mut graph = Graph::open_in_memory().unwrap();
    graph.materialize_from(&store).unwrap();
    graph
}

#[test]
fn agent_authority_may_link_self_to_a_person() {
    // A relationship the agent has to a person is an ordinary link, not a self-model content write, so
    // it is permitted under Agent authority (a maintenance pass, or a platform turn). The link folds:
    // reading `self`'s links surfaces the person, and removing it clears the edge.
    let (seed, self_id, rowan) = self_and_person_events();
    let clock = ManualClock::new(Timestamp::from_millis(2_000));
    let mut writer = block(materialize(&seed), clock, Teller::Agent, Authority::Agent);
    writer
        .link(self_id, rowan, RelationName::Knows, None)
        .unwrap();
    let created = writer.into_effects().events;
    assert!(
        created.iter().any(|event| matches!(
            event,
            EventPayload::LinkCreated { from, relation, .. }
                if *from == self_id && *relation == RelationName::Knows
        )),
        "a self knows-link is authored, not barred",
    );

    // Fold the create over the committed state and read `self`'s links: the person surfaces.
    let mut with_link = seed.clone();
    with_link.extend(created);
    let clock = ManualClock::new(Timestamp::from_millis(3_000));
    let mut reader = block(
        materialize(&with_link),
        clock,
        Teller::Agent,
        Authority::Agent,
    );
    let links = reader.links(self_id).unwrap();
    assert!(
        links.iter().any(|link| link.other == rowan),
        "self's knows-link to the person folds into the link reader",
    );

    // Removing it succeeds and clears the edge.
    let clock = ManualClock::new(Timestamp::from_millis(4_000));
    let mut remover = block(
        materialize(&with_link),
        clock,
        Teller::Agent,
        Authority::Agent,
    );
    remover.unlink(self_id, rowan, RelationName::Knows).unwrap();
    let removed = remover.into_effects().events;
    assert!(
        removed
            .iter()
            .any(|event| matches!(event, EventPayload::LinkRemoved { .. })),
        "the self knows-link is removable",
    );
    let mut without_link = with_link.clone();
    without_link.extend(removed);
    let clock = ManualClock::new(Timestamp::from_millis(5_000));
    let mut reader = block(
        materialize(&without_link),
        clock,
        Teller::Agent,
        Authority::Agent,
    );
    assert!(
        reader.links(self_id).unwrap().is_empty(),
        "removing the self knows-link clears it from the reader",
    );
}

#[test]
fn a_same_as_naming_self_is_refused_under_every_authority() {
    // A `same_as` binds two references to one identity; naming `self` folds the agent into a person's
    // identity class, a category error refused under every authority — the operator included. `rowan` is
    // a bare profile (no entries), so under Agent authority this is exactly the free-merge empty-profile
    // path that would otherwise assert the `same_as` directly; the bar fires ahead of it. Both endpoint
    // orderings are refused.
    let (seed, self_id, rowan) = self_and_person_events();
    for authority in [Authority::Agent, Authority::Operator, Authority::Platform] {
        let clock = ManualClock::new(Timestamp::from_millis(2_000));
        let mut from_self = block(materialize(&seed), clock, Teller::Agent, authority);
        assert!(
            matches!(
                from_self
                    .link(self_id, rowan, RelationName::SameAs, None)
                    .unwrap_err(),
                MemoryError::SelfMergeForbidden
            ),
            "self on the from side is refused under {authority:?}",
        );

        let clock = ManualClock::new(Timestamp::from_millis(2_000));
        let mut to_self = block(materialize(&seed), clock, Teller::Agent, authority);
        assert!(
            matches!(
                to_self
                    .link(rowan, self_id, RelationName::SameAs, None)
                    .unwrap_err(),
                MemoryError::SelfMergeForbidden
            ),
            "self on the to side is refused under {authority:?}",
        );
    }
}

#[test]
fn a_propose_merge_naming_self_is_refused() {
    // The proposal path is barred too: the operator cannot confirm a merge that should never have been
    // proposed, so it never reaches the buffer.
    let (seed, self_id, rowan) = self_and_person_events();
    let clock = ManualClock::new(Timestamp::from_millis(2_000));
    let mut proposer = block(
        materialize(&seed),
        clock,
        Teller::Agent,
        Authority::Platform,
    );
    assert!(matches!(
        proposer.propose_merge(self_id, rowan, None).unwrap_err(),
        MemoryError::SelfMergeForbidden
    ));
}
