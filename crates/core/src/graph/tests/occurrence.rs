//! Bi-temporal `occurred_at` (Stage 9): the materializer denormalizes each entry's typed occurrence
//! into the sortable `occurred_sort` column read-side views expose.

use super::materialized;
use crate::{
    event::{EventPayload, Teller, Visibility},
    graph::Graph,
    ids::{EntryId, MemoryId, MemoryName, Namespace},
    time::{BEFORE_AFTER_EPSILON_MILLIS, CivilDate, Rrule, TemporalRef, Timestamp},
};

const DAY: i64 = 86_400_000;

fn created(id: MemoryId, name: impl Into<MemoryName>) -> EventPayload {
    EventPayload::memory_created(id, name)
}

fn appended(id: MemoryId, entry_id: EntryId, occurred_at: Option<TemporalRef>) -> EventPayload {
    EventPayload::MemoryContentAppended {
        id,
        entry_id,
        asserted_at: Timestamp::from_millis(1),
        occurred_at,
        text: "fact".to_owned(),
        told_by: Teller::Agent,
        told_in: None,
        visibility: Visibility::Public,
    }
}

#[test]
fn denormalizes_each_variant_to_its_representative_instant() {
    let id = MemoryId::generate();
    let refs = [
        Some(TemporalRef::Instant(Timestamp::from_millis(1_000))),
        Some(TemporalRef::Day(CivilDate("2026-06-03".into()))),
        Some(TemporalRef::Approx {
            center: Timestamp::from_millis(10 * DAY),
            fuzz_days: 2,
        }),
        Some(TemporalRef::Range {
            start: Timestamp::from_millis(0),
            end: Timestamp::from_millis(100),
        }),
        // A recurring rule has no fixed instant, and a plain entry no occurrence at all: both NULL.
        Some(TemporalRef::Recurring(Rrule("FREQ=WEEKLY".into()))),
        None,
    ];
    let entry_ids: Vec<EntryId> = refs.iter().map(|_| EntryId::generate()).collect();
    let mut events = vec![created(id, Namespace::Topic.with_name("dated"))];
    for (entry_id, reference) in entry_ids.iter().zip(refs.iter()) {
        events.push(appended(id, *entry_id, reference.clone()));
    }

    let (_store, graph) = materialized(events);
    let entries = graph.entries_local(id).unwrap();
    assert_eq!(entries.len(), refs.len());
    for (entry, reference) in entries.iter().zip(refs.iter()) {
        let expected = reference
            .as_ref()
            .and_then(|r| r.bounds(None, BEFORE_AFTER_EPSILON_MILLIS).sort);
        assert_eq!(entry.occurred_sort, expected);
    }
}

#[test]
fn before_after_resolves_against_its_anchor() {
    let anchor = MemoryId::generate();
    let dependent = MemoryId::generate();
    let anchor_at = 1_000_000;
    let dep_entry = EntryId::generate();
    let (_store, graph) = materialized(vec![
        created(anchor, Namespace::Event.with_name("wedding")),
        appended(
            anchor,
            EntryId::generate(),
            Some(TemporalRef::Instant(Timestamp::from_millis(anchor_at))),
        ),
        created(dependent, Namespace::Event.with_name("reception")),
        appended(
            dependent,
            dep_entry,
            Some(TemporalRef::after(Namespace::Event.with_name("wedding"))),
        ),
    ]);
    let entries = graph.entries_local(dependent).unwrap();
    assert_eq!(
        entries[0].occurred_sort,
        Some(Timestamp::from_millis(
            anchor_at + BEFORE_AFTER_EPSILON_MILLIS
        ))
    );
}

#[test]
fn before_after_resolves_a_soft_deleted_anchor() {
    // MemoryDeleted preserves contents, so an anchor deleted before the dependent's append still
    // resolves — the spec's load-bearing watch-list case.
    let anchor = MemoryId::generate();
    let dependent = MemoryId::generate();
    let anchor_at = 2_000_000;
    let (_store, graph) = materialized(vec![
        created(anchor, Namespace::Event.with_name("move")),
        appended(
            anchor,
            EntryId::generate(),
            Some(TemporalRef::Instant(Timestamp::from_millis(anchor_at))),
        ),
        EventPayload::memory_deleted(anchor),
        created(dependent, Namespace::Event.with_name("housewarming")),
        appended(
            dependent,
            EntryId::generate(),
            Some(TemporalRef::after(Namespace::Event.with_name("move"))),
        ),
    ]);
    let entries = graph.entries_local(dependent).unwrap();
    assert_eq!(
        entries[0].occurred_sort,
        Some(Timestamp::from_millis(
            anchor_at + BEFORE_AFTER_EPSILON_MILLIS
        ))
    );
}

#[test]
fn before_after_with_an_unknown_anchor_is_untimed() {
    let dependent = MemoryId::generate();
    let (_store, graph) = materialized(vec![
        created(dependent, Namespace::Event.with_name("orphan")),
        appended(
            dependent,
            EntryId::generate(),
            Some(TemporalRef::after(
                Namespace::Event.with_name("never-created"),
            )),
        ),
    ]);
    let entries = graph.entries_local(dependent).unwrap();
    assert_eq!(entries[0].occurred_sort, None);
}

#[test]
fn occurrence_columns_survive_replay() {
    let id = MemoryId::generate();
    let (store, graph) = materialized(vec![
        created(id, Namespace::Topic.with_name("dated")),
        appended(
            id,
            EntryId::generate(),
            Some(TemporalRef::Day(CivilDate("2026-06-03".into()))),
        ),
    ]);
    // A second projection of the same log reproduces the denormalized columns exactly.
    let mut replayed = Graph::open_in_memory().unwrap();
    replayed.materialize_from(&store).unwrap();
    let original: Vec<_> = graph
        .entries_local(id)
        .unwrap()
        .iter()
        .map(|e| e.occurred_sort)
        .collect();
    let again: Vec<_> = replayed
        .entries_local(id)
        .unwrap()
        .iter()
        .map(|e| e.occurred_sort)
        .collect();
    assert_eq!(original, again);
    assert!(original[0].is_some());
}

#[test]
fn occurrences_in_window_returns_in_window_pairs_soonest_first() {
    let a = MemoryId::generate();
    let b = MemoryId::generate();
    let c = MemoryId::generate();
    let name_a = Namespace::Event.with_name("a");
    let name_b = Namespace::Event.with_name("b");
    let (_store, graph) = materialized(vec![
        created(a, &name_a),
        appended(
            a,
            EntryId::generate(),
            Some(TemporalRef::Instant(Timestamp::from_millis(300))),
        ),
        // An untimed entry on the same memory must not pull it in twice.
        appended(a, EntryId::generate(), None),
        created(b, &name_b),
        appended(
            b,
            EntryId::generate(),
            Some(TemporalRef::Instant(Timestamp::from_millis(100))),
        ),
        created(c, Namespace::Event.with_name("c")),
        appended(
            c,
            EntryId::generate(),
            Some(TemporalRef::Instant(Timestamp::from_millis(5_000))),
        ),
    ]);
    let hits = graph
        .occurrences_in_window(Timestamp::from_millis(0), Timestamp::from_millis(1_000))
        .unwrap();
    let names: Vec<_> = hits
        .iter()
        .map(|(memory, _)| memory.name.as_str().to_owned())
        .collect();
    // Ordered by occurrence (100 then 300); c at 5_000 is out of the window.
    assert_eq!(names, vec![name_b.to_string(), name_a.to_string()]);
}

#[test]
fn occurrences_in_window_excludes_soft_deleted() {
    let a = MemoryId::generate();
    let (_store, graph) = materialized(vec![
        created(a, Namespace::Event.with_name("a")),
        appended(
            a,
            EntryId::generate(),
            Some(TemporalRef::Instant(Timestamp::from_millis(100))),
        ),
        EventPayload::memory_deleted(a),
    ]);
    assert!(
        graph
            .occurrences_in_window(Timestamp::from_millis(0), Timestamp::from_millis(1_000))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn recurring_memories_lists_only_true_recurrences() {
    let standup = MemoryId::generate();
    let concrete = MemoryId::generate();
    let dangling = MemoryId::generate();
    let standup_name = Namespace::Event.with_name("standup");
    let (_store, graph) = materialized(vec![
        created(standup, &standup_name),
        appended(
            standup,
            EntryId::generate(),
            Some(TemporalRef::Recurring(Rrule("FREQ=WEEKLY".into()))),
        ),
        created(concrete, Namespace::Event.with_name("concrete")),
        appended(
            concrete,
            EntryId::generate(),
            Some(TemporalRef::Day(CivilDate("2026-06-03".into()))),
        ),
        // An unresolved BeforeAfter is also sort-null but is not a recurrence.
        created(dangling, Namespace::Event.with_name("dangling")),
        appended(
            dangling,
            EntryId::generate(),
            Some(TemporalRef::after(
                Namespace::Event.with_name("never-created"),
            )),
        ),
    ]);
    let names: Vec<_> = graph
        .recurring_memories()
        .unwrap()
        .iter()
        .map(|memory| memory.name.as_str().to_owned())
        .collect();
    assert_eq!(names, vec![standup_name.to_string()]);
}

#[test]
fn recurring_entries_lists_live_recurring_entries_by_memory() {
    let (mem_a, mem_b) = (MemoryId::generate(), MemoryId::generate());
    let (e1, e2, e3, e4) = (
        EntryId::generate(),
        EntryId::generate(),
        EntryId::generate(),
        EntryId::generate(),
    );
    let (_store, graph) = materialized(vec![
        created(mem_a, Namespace::Person.with_name("rowan")),
        created(mem_b, Namespace::Event.with_name("standup")),
        // A live recurring entry on mem_a, and a non-recurring one (excluded).
        appended(
            mem_a,
            e1,
            Some(TemporalRef::Recurring(Rrule("FREQ=WEEKLY".into()))),
        ),
        appended(
            mem_a,
            e2,
            Some(TemporalRef::Instant(Timestamp::from_millis(1_000))),
        ),
        // A recurring entry on mem_b that is superseded (excluded), then a live one.
        appended(
            mem_b,
            e3,
            Some(TemporalRef::Recurring(Rrule("FREQ=DAILY".into()))),
        ),
        EventPayload::MemorySuperseded {
            id: mem_b,
            entry: e3,
            superseded_by: e4,
        },
        appended(
            mem_b,
            e4,
            Some(TemporalRef::Recurring(Rrule("FREQ=MONTHLY".into()))),
        ),
    ]);

    // Only the two live recurring entries, each under its memory — the instant and the superseded one
    // are absent.
    let recurring = graph.recurring_entries().unwrap();
    let by_memory: std::collections::BTreeMap<_, _> = recurring
        .iter()
        .map(|entry| (entry.memory, entry.rrule.as_str()))
        .collect();
    assert_eq!(recurring.len(), 2);
    assert_eq!(by_memory.get(&mem_a).copied(), Some("FREQ=WEEKLY"));
    assert_eq!(by_memory.get(&mem_b).copied(), Some("FREQ=MONTHLY"));
}
