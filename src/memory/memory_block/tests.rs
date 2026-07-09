use super::{AppendOptions, Authority, MemoryBlock, MemoryError, VisibilityChoice};
use crate::{
    clock::ManualClock,
    engine::Engine,
    event::{Cardinality, EventPayload, LinkSource, MergeProposalSource, Teller, Visibility},
    graph::{Graph, GraphError},
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
        None,
        Vec::new(),
        TEST_MAX_ENTRY_CHARS,
    )
    .unwrap()
}

/// The character limit the test block enforces — generous enough that existing test content passes,
/// while still exercising the limit in the dedicated oversized-content tests.
const TEST_MAX_ENTRY_CHARS: usize = 10_000;

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
        .link(a, b, RelationName::Other("bogus_relation".into()), None)
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
    // The speaker (the teller) is not the subject of person/marcus, so the default is private.
    let marcus = block
        .create(Namespace::Person.with_name("marcus"), None)
        .unwrap();
    block
        .append(marcus, "is being managed out", AppendOptions::default())
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
    let dave_discord = block
        .create(Namespace::Person.with_name("dave@discord"), None)
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
        .link(dave, dave_discord, RelationName::SameAs, None)
        .unwrap();

    // A retraction, by contrast, stays operator-only — the agent can neither assert nor undo a merge.
    assert!(matches!(
        block
            .unlink(dave, dave_discord, RelationName::SameAs)
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
            } if *from == dave && *to == dave_discord
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
    let dave_discord = block
        .create(Namespace::Person.with_name("dave@discord"), None)
        .unwrap();

    block
        .link(dave, dave_discord, RelationName::SameAs, None)
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
                visibility: Some(VisibilityChoice::Private),
                ..AppendOptions::default()
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

#[test]
fn revise_rolls_back_the_append_when_the_supersede_fails() {
    // revise is append-then-supersede; a failed supersede must not leave the append's buffered event
    // behind. Without the transaction, a caught error (a Lua `pcall`) would commit the orphaned new
    // entry beside the stale value it was meant to replace. The transaction rolls the buffer back to
    // before the append.
    let graph = Graph::open_in_memory().unwrap();
    let clock = ManualClock::new(Timestamp::from_millis(1_000));
    let mut block = block(graph, clock, Teller::Agent, Authority::Platform);
    let a = block.create(Namespace::Topic.with_name("a"), None).unwrap();
    block
        .append(a, "original", AppendOptions::default())
        .unwrap();
    // A foreign entry (from a different memory) is not a live entry of `a`, so the supersede fails.
    let b = block.create(Namespace::Topic.with_name("b"), None).unwrap();
    let foreign = block
        .append(b, "b content", AppendOptions::default())
        .unwrap();
    let error = block
        .revise(a, foreign, "replacement", AppendOptions::default())
        .unwrap_err();
    assert!(
        matches!(error, MemoryError::UnknownEntry(_)),
        "revise should fail on a foreign entry, got {error:?}"
    );
    // The revise's append was rolled back: only the original append remains on `a`.
    let effects = block.into_effects();
    let appended: Vec<&EventPayload> = effects
        .events
        .iter()
        .filter(|event| matches!(event, EventPayload::MemoryContentAppended { id, .. } if *id == a))
        .collect();
    assert_eq!(
        appended.len(),
        1,
        "the failed revise's append should have been rolled back, but found {appended:?}"
    );
}

#[test]
fn graph_error_carries_a_memory_context_prefix() {
    // The Graph variant is infrastructure — `route_error` intercepts it and surfaces a generic
    // "internal graph error" to the agent — so its Display follows the error-display convention: a
    // `memory:` layer prefix nesting the graph error's own `materialized graph (…)` prefix, so a
    // propagated error reads as nested context (`memory: materialized graph (malformed): …`).
    let error = MemoryError::Graph(GraphError::Malformed("a corrupt id".to_owned()));
    assert_eq!(
        error.to_string(),
        "memory: materialized graph (malformed): a corrupt id"
    );
}

/// A block with a custom `max_entry_chars` limit, for the oversized-content tests.
fn block_with_limit(graph: Graph, clock: ManualClock, max_entry_chars: usize) -> MemoryBlock {
    let engine = Engine::new(Box::new(MemoryStore::new()), graph, Box::new(clock));
    MemoryBlock::new(
        engine,
        Teller::Agent,
        Authority::Platform,
        ConversationId::generate(),
        None,
        Vec::new(),
        max_entry_chars,
    )
    .unwrap()
}

#[test]
fn content_too_long_display_message_names_length_and_limit() {
    // The teachable message names the entry's length and the limit, and guides the agent to
    // summarize — so the agent reads the cause and corrects rather than guessing.
    let error = MemoryError::ContentTooLong {
        length: 2048,
        limit: 1000,
    };
    let message = error.to_string();
    assert!(
        message.contains("2048"),
        "the message should name the entry's length: {message}"
    );
    assert!(
        message.contains("1000"),
        "the message should name the limit: {message}"
    );
    assert!(
        message.contains("summarize"),
        "the message should guide the agent to summarize: {message}"
    );
}

#[test]
fn append_rejects_oversized_content() {
    let graph = Graph::open_in_memory().unwrap();
    let clock = ManualClock::new(Timestamp::from_millis(1_000));
    let mut block = block_with_limit(graph, clock, 10);
    let topic = block.create(Namespace::Topic.with_name("a"), None).unwrap();
    let oversized = "x".repeat(11);
    let error = block
        .append(topic, &oversized, AppendOptions::default())
        .unwrap_err();
    assert!(
        matches!(error, MemoryError::ContentTooLong { length, limit } if length == 11 && limit == 10),
        "expected ContentTooLong with length 11 and limit 10, got {error:?}"
    );
    // Nothing was buffered — the rejection happened before the push.
    let effects = block.into_effects();
    assert!(
        !effects
            .events
            .iter()
            .any(|event| matches!(event, EventPayload::MemoryContentAppended { .. })),
        "no content entry should have been buffered"
    );
}

#[test]
fn append_accepts_at_limit() {
    let graph = Graph::open_in_memory().unwrap();
    let clock = ManualClock::new(Timestamp::from_millis(1_000));
    let mut block = block_with_limit(graph, clock, 10);
    let topic = block.create(Namespace::Topic.with_name("a"), None).unwrap();
    // Exactly at the limit — not exceeding it — so the append succeeds.
    let at_limit = "x".repeat(10);
    let result = block.append(topic, &at_limit, AppendOptions::default());
    assert!(result.is_ok(), "an entry at the limit should be accepted");
}

#[test]
fn create_rejects_oversized_first_entry() {
    let graph = Graph::open_in_memory().unwrap();
    let clock = ManualClock::new(Timestamp::from_millis(1_000));
    let mut block = block_with_limit(graph, clock, 10);
    let oversized = "x".repeat(11);
    let error = block
        .create(Namespace::Topic.with_name("a"), Some(&oversized))
        .unwrap_err();
    assert!(
        matches!(error, MemoryError::ContentTooLong { length, limit } if length == 11 && limit == 10),
        "expected ContentTooLong, got {error:?}"
    );
    // The create ran in a transaction, so the rolled-back create leaves the buffer empty of
    // MemoryCreated events.
    let effects = block.into_effects();
    assert!(
        !effects
            .events
            .iter()
            .any(|event| matches!(event, EventPayload::MemoryCreated { .. })),
        "no MemoryCreated should have been buffered after a rolled-back create"
    );
}

#[test]
fn revise_rejects_oversized_replacement() {
    let graph = Graph::open_in_memory().unwrap();
    let clock = ManualClock::new(Timestamp::from_millis(1_000));
    let mut block = block_with_limit(graph, clock, 10);
    let topic = block.create(Namespace::Topic.with_name("a"), None).unwrap();
    let original = block
        .append(topic, "original", AppendOptions::default())
        .unwrap();
    let oversized = "x".repeat(11);
    let error = block
        .revise(topic, original, &oversized, AppendOptions::default())
        .unwrap_err();
    assert!(
        matches!(error, MemoryError::ContentTooLong { length, limit } if length == 11 && limit == 10),
        "expected ContentTooLong, got {error:?}"
    );
    // The revise ran in a transaction, so the oversized append was rolled back — only the original
    // entry remains on the memory.
    let effects = block.into_effects();
    let appended: Vec<&EventPayload> = effects
        .events
        .iter()
        .filter(
            |event| matches!(event, EventPayload::MemoryContentAppended { id, .. } if *id == topic),
        )
        .collect();
    assert_eq!(
        appended.len(),
        1,
        "the failed revise's append should have been rolled back, leaving only the original"
    );
}
