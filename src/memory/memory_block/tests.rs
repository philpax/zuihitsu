use super::{AppendOptions, Authority, MemoryBlock, MemoryError, VisibilityChoice};
use crate::{
    clock::ManualClock,
    engine::Engine,
    event::{Cardinality, EventPayload, LinkSource, Teller, Visibility},
    graph::Graph,
    ids::{ConversationId, MemoryId, MemoryName, Namespace, NamespacedMemoryName},
    store::{MemoryStore, Store},
    time::{Rrule, TemporalRef, Timestamp},
    vocabulary::RelationName,
};

/// A block over an empty in-memory graph and a conversation with no context — enough to exercise
/// the write invariants directly, no Lua VM and no store materialization involved. The engine's
/// store is a throwaway: these tests read `into_effects` and never commit.
fn block(graph: Graph, clock: ManualClock, teller: Teller, authority: Authority) -> MemoryBlock {
    let engine = Engine::new(Box::new(MemoryStore::new()), graph, Box::new(clock));
    MemoryBlock::new(
        engine,
        teller,
        authority,
        ConversationId::generate(),
        Vec::new(),
    )
    .unwrap()
}

/// A graph seeded with the `self` memory and the `created_by` and `same_as` relations — the
/// minimum to exercise the self-write and merge guards, which key on the resolved `self` id and on
/// the relation. Returns the graph and `self`'s id.
fn graph_with_self() -> (Graph, MemoryId) {
    let mut store = MemoryStore::new();
    let self_id = MemoryId::generate();
    store
        .append(
            Timestamp::from_millis(1_000),
            vec![
                EventPayload::memory_created(self_id, MemoryName::new(MemoryName::SELF)),
                EventPayload::LinkTypeRegistered {
                    name: RelationName::CreatedBy,
                    inverse: RelationName::Created,
                    from_card: Cardinality::One,
                    to_card: Cardinality::Many,
                    symmetric: false,
                    reflexive: false,
                },
                EventPayload::LinkTypeRegistered {
                    name: RelationName::SameAs,
                    inverse: RelationName::SameAs,
                    from_card: Cardinality::Many,
                    to_card: Cardinality::Many,
                    symmetric: true,
                    reflexive: false,
                },
            ],
        )
        .unwrap();
    let mut graph = Graph::open_in_memory().unwrap();
    graph.materialize_from(&store).unwrap();
    (graph, self_id)
}

#[test]
fn create_rejects_a_duplicate_name() {
    let graph = Graph::open_in_memory().unwrap();
    let clock = ManualClock::new(Timestamp::from_millis(1_000));
    let mut block = block(graph, clock, Teller::Agent, Authority::Platform);
    let plan = Namespace::Topic.with_name("plan");
    block.create(&plan, None).unwrap();
    // Caught against the block's own pending create (read-your-writes), before any commit.
    let error = block.create(&plan, None).unwrap_err();
    assert!(matches!(error, MemoryError::NameExists(_)));
}

#[test]
fn link_rejects_an_unregistered_relation() {
    let graph = Graph::open_in_memory().unwrap();
    let clock = ManualClock::new(Timestamp::from_millis(1_000));
    let mut block = block(graph, clock, Teller::Agent, Authority::Platform);
    let a = block.create(Namespace::Topic.with_name("a"), None).unwrap();
    let b = block.create(Namespace::Topic.with_name("b"), None).unwrap();
    let error = block
        .link(a, b, RelationName::Other("bogus_relation".into()))
        .unwrap_err();
    assert!(matches!(error, MemoryError::UnknownRelation(_)));
}

#[test]
fn an_aside_about_another_person_defaults_private() {
    let graph = Graph::open_in_memory().unwrap();
    let clock = ManualClock::new(Timestamp::from_millis(1_000));
    let speaker = MemoryId::generate();
    let mut block = block(
        graph,
        clock,
        Teller::Participant(speaker),
        Authority::Platform,
    );
    // The speaker (the teller) is not the subject of person/phil, so the default is private.
    let phil = block
        .create(Namespace::Person.with_name("phil"), None)
        .unwrap();
    block
        .append(phil, "is being managed out", AppendOptions::default())
        .unwrap();

    let visibility = block
        .into_effects()
        .events
        .into_iter()
        .find_map(|event| match event {
            EventPayload::MemoryContentAppended { visibility, .. } => Some(visibility),
            _ => None,
        })
        .unwrap();
    assert_eq!(visibility, Visibility::PrivateToTeller);
}

#[test]
fn platform_authority_cannot_write_self() {
    let (graph, self_id) = graph_with_self();
    let clock = ManualClock::new(Timestamp::from_millis(2_000));
    let mut block = block(graph, clock, Teller::Agent, Authority::Platform);
    let other = block
        .create(Namespace::Person.with_name("phil"), None)
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
            .link(self_id, other, RelationName::CreatedBy)
            .unwrap_err(),
        MemoryError::SelfWriteForbidden
    ));
    assert!(matches!(
        block
            .link(other, self_id, RelationName::CreatedBy)
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
            vec![
                EventPayload::memory_created(operator_id, NamespacedMemoryName::operator()),
                EventPayload::LinkTypeRegistered {
                    name: RelationName::SameAs,
                    inverse: RelationName::SameAs,
                    from_card: Cardinality::Many,
                    to_card: Cardinality::Many,
                    symmetric: true,
                    reflexive: false,
                },
            ],
        )
        .unwrap();
    let mut graph = Graph::open_in_memory().unwrap();
    graph.materialize_from(&store).unwrap();

    let clock = ManualClock::new(Timestamp::from_millis(2_000));
    let mut block = block(graph, clock, Teller::Agent, Authority::Operator);
    let real = block
        .create(Namespace::Person.with_name("phil"), None)
        .unwrap();

    // Recording content on the anchor is barred — even under operator authority.
    assert!(matches!(
        block
            .append(operator_id, "Real name is Phil", AppendOptions::default())
            .unwrap_err(),
        MemoryError::OperatorWriteForbidden
    ));
    // The merge link to the anchor is not content, so it is allowed.
    assert!(block.link(operator_id, real, RelationName::SameAs).is_ok());
}

#[test]
fn operator_authority_may_write_self_and_links_carry_operator() {
    let (graph, self_id) = graph_with_self();
    let clock = ManualClock::new(Timestamp::from_millis(2_000));
    let mut block = block(graph, clock, Teller::Agent, Authority::Operator);
    let phil = block
        .create(Namespace::Person.with_name("phil"), None)
        .unwrap();

    // The same writes that platform authority bars all succeed from the console.
    block
        .append(
            self_id,
            "I exist to keep Phil's memory.",
            AppendOptions::default(),
        )
        .unwrap();
    block.link(self_id, phil, RelationName::CreatedBy).unwrap();

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
fn platform_authority_cannot_assert_a_same_as_merge() {
    let (graph, _self_id) = graph_with_self();
    let clock = ManualClock::new(Timestamp::from_millis(2_000));
    let mut block = block(graph, clock, Teller::Agent, Authority::Platform);
    let dave = block
        .create(Namespace::Person.with_name("dave"), None)
        .unwrap();
    let dave_discord = block
        .create(Namespace::Person.with_name("dave@discord"), None)
        .unwrap();

    // Merging two identities — or splitting one — is operator-only, regardless of the endpoints.
    assert!(matches!(
        block
            .link(dave, dave_discord, RelationName::SameAs)
            .unwrap_err(),
        MemoryError::MergeForbidden
    ));
    assert!(matches!(
        block
            .unlink(dave, dave_discord, RelationName::SameAs)
            .unwrap_err(),
        MemoryError::MergeForbidden
    ));
}

#[test]
fn operator_authority_may_assert_a_same_as_merge() {
    let (graph, _self_id) = graph_with_self();
    let clock = ManualClock::new(Timestamp::from_millis(2_000));
    let mut block = block(graph, clock, Teller::Agent, Authority::Operator);
    let dave = block
        .create(Namespace::Person.with_name("dave"), None)
        .unwrap();
    let dave_discord = block
        .create(Namespace::Person.with_name("dave@discord"), None)
        .unwrap();

    block
        .link(dave, dave_discord, RelationName::SameAs)
        .unwrap();
}

#[test]
fn agent_authored_writes_about_a_person_require_explicit_visibility() {
    let graph = Graph::open_in_memory().unwrap();
    let clock = ManualClock::new(Timestamp::from_millis(1_000));
    let mut block = block(graph, clock, Teller::Agent, Authority::Platform);

    // An agent-authored entry about a person has no protective default, so it must be classified:
    // both a create-with-content and a bare append fail teachably without an explicit visibility.
    let erin = Namespace::Person.with_name("erin");
    assert!(matches!(
        block
            .create(&erin, Some("may be leaving the team"))
            .unwrap_err(),
        MemoryError::VisibilityRequired
    ));
    let erin = block.create(&erin, None).unwrap();
    assert!(matches!(
        block
            .append(erin, "may be leaving the team", AppendOptions::default())
            .unwrap_err(),
        MemoryError::VisibilityRequired
    ));

    // Once classified it succeeds; and a non-person memory has no subject to guard, so the agent's
    // write there keeps the public default with no classification required.
    block
        .append(
            erin,
            "may be leaving the team",
            AppendOptions {
                by_agent: false,
                visibility: Some(VisibilityChoice::Private),
                occurred_at: None,
                volatility: None,
            },
        )
        .unwrap();
    let roadmap = block
        .create(
            Namespace::Topic.with_name("roadmap"),
            Some("ship on Friday"),
        )
        .unwrap();
    block
        .append(roadmap, "migration first", AppendOptions::default())
        .unwrap();
}

#[test]
fn append_rejects_an_unsupported_recurrence_with_a_teachable_error() {
    // A free-phrased cadence the model emits in place of an rrule arms no wake-up, so the write is
    // rejected for the agent to reissue — surfaced as a teachable error, not swallowed.
    let mut block = block(
        Graph::open_in_memory().unwrap(),
        ManualClock::new(Timestamp::from_millis(1_000)),
        Teller::Agent,
        Authority::Platform,
    );
    let standup = block
        .create(Namespace::Event.with_name("standup"), None)
        .unwrap();
    let err = block
        .append(
            standup,
            "every Monday",
            AppendOptions {
                occurred_at: Some(TemporalRef::Recurring(Rrule("every Monday".into()))),
                ..AppendOptions::default()
            },
        )
        .unwrap_err();
    assert!(
        matches!(err, MemoryError::UnsupportedRecurrence(ref rule) if rule == "every Monday"),
        "{err:?}"
    );
    assert!(
        err.to_string().contains("FREQ"),
        "the error should point at a supported rule: {err}"
    );

    // A supported rule is accepted, and arms a wake-up the scheduler can derive.
    block
        .append(
            standup,
            "team standup",
            AppendOptions {
                occurred_at: Some(TemporalRef::Recurring(Rrule("FREQ=WEEKLY;BYDAY=MO".into()))),
                ..AppendOptions::default()
            },
        )
        .unwrap();
}
