//! Per-memory described-state: content marks a memory stale, a `DescribePassCompleted` clears it, the
//! genesis marker baselines everything that exists at genesis, and the temporal-extraction window is
//! bounded by the memory's `last_described_seq`.

use super::materialized;
use crate::{
    event::{EventPayload, EventSource, Teller, Visibility},
    ids::{EntryId, MemoryId, Namespace, Seq},
    store::{MemoryStore, Store},
    time::Timestamp,
};

fn appended(id: MemoryId, entry_id: EntryId, occurred: bool) -> EventPayload {
    EventPayload::MemoryContentAppended {
        id,
        entry_id,
        asserted_at: Timestamp::from_millis(900),
        occurred_at: occurred.then(|| crate::time::TemporalRef::Instant(Timestamp::from_millis(1))),
        text: "a fact".to_owned(),
        told_by: Teller::Agent,
        told_in: None,
        visibility: Visibility::Public,
    }
}

#[test]
fn content_marks_a_memory_stale_and_a_describe_pass_clears_it() {
    let dave = MemoryId::generate();
    let (mut store, mut graph) = materialized(vec![
        EventPayload::memory_created(dave, Namespace::Person.with_name("dave")),
        appended(dave, EntryId::generate(), false),
    ]);
    assert_eq!(graph.stale_memories().unwrap(), vec![dave]);
    assert_eq!(graph.stale_memory_count().unwrap(), 1);

    // A describer pass over dave clears the staleness.
    store
        .append(
            Timestamp::from_millis(1_100),
            EventSource::Agent,
            vec![EventPayload::describe_pass_completed(vec![dave])],
        )
        .unwrap();
    graph.materialize_from(&store).unwrap();
    assert!(graph.stale_memories().unwrap().is_empty());
    assert_eq!(graph.stale_memory_count().unwrap(), 0);

    // New content re-marks it stale.
    store
        .append(
            Timestamp::from_millis(1_200),
            EventSource::Agent,
            vec![appended(dave, EntryId::generate(), false)],
        )
        .unwrap();
    graph.materialize_from(&store).unwrap();
    assert_eq!(graph.stale_memories().unwrap(), vec![dave]);
}

#[test]
fn genesis_baselines_everything_that_exists_at_genesis() {
    // A memory seeded with content before `GenesisCompleted` is baselined as described, so it is not
    // stale after genesis; a memory created after the marker is stale.
    let seeded = MemoryId::generate();
    let later = MemoryId::generate();
    let (_store, graph) = materialized(vec![
        EventPayload::memory_created(seeded, Namespace::Person.with_name("seeded")),
        appended(seeded, EntryId::generate(), false),
        EventPayload::genesis_completed("hash", Default::default()),
        EventPayload::memory_created(later, Namespace::Person.with_name("dave")),
        appended(later, EntryId::generate(), false),
    ]);
    let stale = graph.stale_memories().unwrap();
    assert_eq!(stale, vec![later], "only the post-genesis memory is stale");
}

#[test]
fn stale_among_narrows_to_the_given_set() {
    let a = MemoryId::generate();
    let b = MemoryId::generate();
    let (_store, graph) = materialized(vec![
        EventPayload::memory_created(a, Namespace::Person.with_name("a")),
        appended(a, EntryId::generate(), false),
        EventPayload::memory_created(b, Namespace::Person.with_name("b")),
        appended(b, EntryId::generate(), false),
    ]);
    // Both are stale, but the narrowed query returns only the requested id.
    assert_eq!(graph.stale_memories_among(&[a]).unwrap(), vec![a]);
    assert!(graph.stale_memories_among(&[]).unwrap().is_empty());
    let mut both = graph.stale_memories_among(&[a, b]).unwrap();
    both.sort();
    let mut expected = vec![a, b];
    expected.sort();
    assert_eq!(both, expected);
}

#[test]
fn untimed_window_excludes_timed_and_already_described_entries() {
    let mem = MemoryId::generate();
    let timed = EntryId::generate();
    let untimed = EntryId::generate();
    let mut store = MemoryStore::new();
    store
        .append(
            Timestamp::from_millis(1_000),
            EventSource::Agent,
            vec![
                EventPayload::memory_created(mem, Namespace::Event.with_name("thing")),
                appended(mem, timed, true),
                appended(mem, untimed, false),
            ],
        )
        .unwrap();
    let mut graph = crate::graph::Graph::open_in_memory().unwrap();
    graph.materialize_from(&store).unwrap();

    // From seq 0, only the untimed entry is eligible — the explicitly-timed one is excluded.
    let (content_seq, _) = graph.described_state(mem).unwrap().unwrap();
    assert_eq!(
        graph.untimed_entries_since(mem, Seq::ZERO).unwrap(),
        vec![untimed]
    );

    // After the pass considers the memory, its window advances past the entries seen, so a re-read at
    // the memory's content watermark yields nothing.
    assert!(
        graph
            .untimed_entries_since(mem, content_seq)
            .unwrap()
            .is_empty()
    );
}
