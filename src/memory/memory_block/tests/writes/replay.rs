//! Deterministic replay and rollback: a redirected write refolds to the same placement, an unsupported
//! recurrence is a teachable error, a failed revise rolls its append back, and the graph error carries
//! its memory-context prefix.

use super::{
    AppendOptions, Authority, MemoryError, VisibilityChoice, block, designated_primary_seed,
    graph_from,
};
use crate::{
    clock::ManualClock,
    event::{EventPayload, Teller},
    graph::{Graph, GraphError},
    ids::Namespace,
    time::{Rrule, TemporalRef, Timestamp},
};

#[test]
fn a_redirected_write_replays_deterministically() {
    // The redirect reads committed state and emits an event carrying the concrete primary id, so
    // refolding the log reproduces the same placement: the entry sits on the primary and reads back
    // across the whole class from either handle.
    let (seed, dave, marcus) = designated_primary_seed();
    let clock = ManualClock::new(Timestamp::from_millis(2_000));
    let mut block = block(
        graph_from(seed.clone()),
        clock,
        Teller::Agent,
        Authority::Platform,
    );
    block
        .append(
            dave,
            "ships on Fridays",
            AppendOptions {
                visibility: Some(VisibilityChoice::Public),
                ..AppendOptions::default()
            },
        )
        .unwrap();
    let mut replayed = seed;
    replayed.extend(block.into_effects().events);
    let graph = graph_from(replayed);

    let on_primary = graph.class_entries(marcus).unwrap();
    assert!(
        on_primary
            .iter()
            .any(|entry| entry.text == "ships on Fridays"),
        "the refolded entry sits on the primary"
    );
    // The clean, non-primary handle reads the same class, so the fact surfaces from it too.
    let from_dave = graph.class_entries(dave).unwrap();
    assert!(
        from_dave
            .iter()
            .any(|entry| entry.text == "ships on Fridays"),
        "the fact reads back across the whole class"
    );
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
