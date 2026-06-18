use super::{AppendOptions, Authority, MemoryBlock, MemoryError, VisibilityChoice};
use crate::{
    clock::ManualClock,
    engine::Engine,
    event::{Cardinality, EventPayload, LinkSource, Teller, Visibility},
    graph::Graph,
    ids::{ConversationId, MemoryId, MemoryName, Namespace},
    store::{MemoryStore, Store},
    time::Timestamp,
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
                EventPayload::MemoryCreated {
                    id: self_id,
                    name: MemoryName::new(MemoryName::SELF),
                },
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
    block
        .create(Namespace::Topic.handle("plan").as_str(), None)
        .unwrap();
    // Caught against the block's own pending create (read-your-writes), before any commit.
    let error = block
        .create(Namespace::Topic.handle("plan").as_str(), None)
        .unwrap_err();
    assert!(matches!(error, MemoryError::NameExists(_)));
}

#[test]
fn link_rejects_an_unregistered_relation() {
    let graph = Graph::open_in_memory().unwrap();
    let clock = ManualClock::new(Timestamp::from_millis(1_000));
    let mut block = block(graph, clock, Teller::Agent, Authority::Platform);
    let a = block
        .create(Namespace::Topic.handle("a").as_str(), None)
        .unwrap();
    let b = block
        .create(Namespace::Topic.handle("b").as_str(), None)
        .unwrap();
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
        .create(Namespace::Person.handle("phil").as_str(), None)
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
        .create(Namespace::Person.handle("phil").as_str(), None)
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
fn operator_authority_may_write_self_and_links_carry_operator() {
    let (graph, self_id) = graph_with_self();
    let clock = ManualClock::new(Timestamp::from_millis(2_000));
    let mut block = block(graph, clock, Teller::Agent, Authority::Operator);
    let phil = block
        .create(Namespace::Person.handle("phil").as_str(), None)
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
        .create(Namespace::Person.handle("dave").as_str(), None)
        .unwrap();
    let dave_discord = block
        .create(Namespace::Person.handle("dave@discord").as_str(), None)
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
        .create(Namespace::Person.handle("dave").as_str(), None)
        .unwrap();
    let dave_discord = block
        .create(Namespace::Person.handle("dave@discord").as_str(), None)
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
    assert!(matches!(
        block
            .create(
                Namespace::Person.handle("erin").as_str(),
                Some("may be leaving the team")
            )
            .unwrap_err(),
        MemoryError::VisibilityRequired
    ));
    let erin = block
        .create(Namespace::Person.handle("erin").as_str(), None)
        .unwrap();
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
            Namespace::Topic.handle("roadmap").as_str(),
            Some("ship on Friday"),
        )
        .unwrap();
    block
        .append(roadmap, "migration first", AppendOptions::default())
        .unwrap();
}
