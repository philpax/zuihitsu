//! Retraction: the reason is mandatory and auditable, the target must be a live entry of the memory,
//! and a buffered retraction drops its entry from the live read at once while history keeps it.

use crate::{
    clock::ManualClock,
    event::{EventPayload, Teller},
    graph::Graph,
    ids::{EntryId, Namespace},
    memory::memory_block::tests::writes::{AppendOptions, Authority, MemoryError, block},
    time::Timestamp,
};

#[test]
fn retract_rejects_an_empty_reason() {
    // A retraction leaves a tombstone in history, so it must say why; an empty or whitespace-only
    // reason is unauditable, a teachable error rather than a silent tombstone.
    let graph = Graph::open_in_memory().unwrap();
    let clock = ManualClock::new(Timestamp::from_millis(1_000));
    let mut block = block(graph, clock, Teller::Agent, Authority::Platform);
    let topic = block
        .create(Namespace::Topic.with_name("plan"), None)
        .unwrap();
    let entry = block
        .append(topic, "a fact", AppendOptions::default())
        .unwrap();
    assert!(matches!(
        block.retract(topic, entry, "", None).unwrap_err(),
        MemoryError::RetractionReasonRequired
    ));
    assert!(matches!(
        block.retract(topic, entry, "   ", None).unwrap_err(),
        MemoryError::RetractionReasonRequired
    ));
}

#[test]
fn retract_rejects_an_unknown_entry() {
    // Retracting an id that is not a live entry of the memory is the same teachable misuse supersede
    // reports — the agent retracts an entry it read from the memory.
    let graph = Graph::open_in_memory().unwrap();
    let clock = ManualClock::new(Timestamp::from_millis(1_000));
    let mut block = block(graph, clock, Teller::Agent, Authority::Platform);
    let topic = block
        .create(Namespace::Topic.with_name("plan"), None)
        .unwrap();
    assert!(matches!(
        block
            .retract(topic, EntryId::generate(), "gone", None)
            .unwrap_err(),
        MemoryError::UnknownEntry(_)
    ));
}

#[test]
fn retract_hides_the_entry_from_a_live_read_and_buffers_the_reason() {
    // A retraction buffered this block drops its entry from the live read at once (read-your-writes),
    // exactly as a supersession does, and records the reason on the `EntryRetracted` event.
    let graph = Graph::open_in_memory().unwrap();
    let clock = ManualClock::new(Timestamp::from_millis(1_000));
    let mut block = block(graph, clock, Teller::Agent, Authority::Platform);
    let topic = block
        .create(Namespace::Topic.with_name("plan"), None)
        .unwrap();
    let kept = block
        .append(topic, "kept fact", AppendOptions::default())
        .unwrap();
    let withdrawn = block
        .append(topic, "withdrawn fact", AppendOptions::default())
        .unwrap();
    block
        .retract(topic, withdrawn, "filed on the wrong topic", None)
        .unwrap();

    // The live read no longer carries the retracted entry, but history keeps it.
    let live: Vec<String> = block
        .entries(topic)
        .unwrap()
        .into_iter()
        .map(|entry| entry.text)
        .collect();
    assert_eq!(live, ["kept fact"]);
    let _ = kept;

    let effects = block.into_effects();
    let retracted = effects.events.iter().find_map(|event| match event {
        EventPayload::EntryRetracted {
            memory,
            entry,
            reason,
            ..
        } => Some((*memory, *entry, reason.clone())),
        _ => None,
    });
    assert_eq!(
        retracted,
        Some((topic, withdrawn, "filed on the wrong topic".to_owned()))
    );
}
