//! The write-path basics and teachable write errors.

use super::{AppendOptions, Authority, MemoryError, VisibilityChoice, block};
use crate::{
    clock::ManualClock,
    event::{EventPayload, Teller, Visibility},
    graph::{Graph, GraphError},
    ids::{MemoryId, Namespace},
    time::{Rrule, TemporalRef, Timestamp},
    vocabulary::RelationName,
};

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
