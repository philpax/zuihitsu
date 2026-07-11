use rusqlite::params;

use super::Graph;
use crate::{
    event::{ArbitrationResolution, Event, EventPayload, EventSource, Teller, Visibility},
    ids::{EntryId, MemoryId, Namespace, Seq},
    time::{BEFORE_AFTER_EPSILON_MILLIS, CivilDate, MILLIS_PER_DAY, Rrule, TemporalRef, Timestamp},
};

fn event(seq: u64, payload: EventPayload) -> Event {
    Event {
        seq: Seq(seq),
        recorded_at: Timestamp::from_millis(1),
        source: EventSource::Agent,
        payload,
    }
}

/// The materializer must write all three denormalized columns from a single `TemporalRef`, in
/// the right slots — `occurred_sort` alone (the only column a read-side view exposes today) can't
/// catch a lo/hi column-order slip, so this asserts against the columns directly.
#[test]
fn occurrence_columns_match_the_derived_bounds() {
    let mut graph = Graph::open_in_memory().unwrap();
    let id = MemoryId::generate();
    let entry = EntryId::generate();
    let occurred = TemporalRef::Day(CivilDate("2026-06-03".into()));
    graph
        .apply(&event(
            1,
            EventPayload::memory_created(id, Namespace::Event.with_name("cleaning")),
        ))
        .unwrap();
    graph
        .apply(&event(
            2,
            EventPayload::MemoryContentAppended {
                id,
                entry_id: entry,
                asserted_at: Timestamp::from_millis(1),
                occurred_at: Some(occurred.clone()),
                text: "scheduled cleaning".to_owned(),
                told_by: Teller::Agent,
                told_in: None,
                visibility: Visibility::Public,
            },
        ))
        .unwrap();

    let columns: (Option<i64>, Option<i64>, Option<i64>) = graph
        .conn
        .query_row(
            "SELECT occurred_sort, occurred_lo, occurred_hi
                 FROM content_entries WHERE entry_id = ?1",
            params![entry.0.to_string()],
            |r| r.try_into(),
        )
        .unwrap();
    let bounds = occurred.bounds(None, 0);
    assert_eq!(columns.0, bounds.sort.map(Timestamp::as_millis));
    assert_eq!(columns.1, bounds.lo.map(Timestamp::as_millis));
    assert_eq!(columns.2, bounds.hi.map(Timestamp::as_millis));
    assert!(columns.1 < columns.0 && columns.0 < columns.2);
}

/// `EntryTemporalResolved` updates an already-appended (untimed) entry's occurrence columns in
/// place, resolving a `BeforeAfter` against the projection just like an explicit occurrence.
#[test]
fn entry_temporal_resolved_updates_columns_in_place() {
    let mut graph = Graph::open_in_memory().unwrap();
    let anchor = MemoryId::generate();
    let dependent = MemoryId::generate();
    let entry = EntryId::generate();
    let anchor_at = 1_000_000;
    let untimed = |id, entry_id| EventPayload::MemoryContentAppended {
        id,
        entry_id,
        asserted_at: Timestamp::from_millis(1),
        occurred_at: None,
        text: "fact".to_owned(),
        told_by: Teller::Agent,
        told_in: None,
        visibility: Visibility::Public,
    };
    let events = [
        EventPayload::memory_created(anchor, Namespace::Event.with_name("wedding")),
        EventPayload::MemoryContentAppended {
            id: anchor,
            entry_id: EntryId::generate(),
            asserted_at: Timestamp::from_millis(1),
            occurred_at: Some(TemporalRef::Instant(Timestamp::from_millis(anchor_at))),
            text: "the wedding".to_owned(),
            told_by: Teller::Agent,
            told_in: None,
            visibility: Visibility::Public,
        },
        EventPayload::memory_created(dependent, Namespace::Event.with_name("reception")),
        untimed(dependent, entry),
    ];
    for (seq, payload) in events.into_iter().enumerate() {
        graph.apply(&event(seq as u64 + 1, payload)).unwrap();
    }
    // The dependent entry starts untimed.
    let sort_before: Option<i64> = graph
        .conn
        .query_row(
            "SELECT occurred_sort FROM content_entries WHERE entry_id = ?1",
            rusqlite::params![entry.0.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(sort_before, None);

    graph
        .apply(&event(
            5,
            EventPayload::entry_temporal_resolved(
                dependent,
                entry,
                TemporalRef::after(Namespace::Event.with_name("wedding")),
                None,
            ),
        ))
        .unwrap();
    let sort_after: Option<i64> = graph
        .conn
        .query_row(
            "SELECT occurred_sort FROM content_entries WHERE entry_id = ?1",
            rusqlite::params![entry.0.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(sort_after, Some(anchor_at + BEFORE_AFTER_EPSILON_MILLIS));
}

/// An unresolved arbitration (crediting neither side) projects its competing entries as disputed;
/// crediting a side clears them, superseding one account drops the dispute (the ≥2-live rule), and
/// a fresh arbitration replaces the prior memory's state.
#[test]
fn disputed_entries_track_the_latest_unresolved_arbitration() {
    let mut graph = Graph::open_in_memory().unwrap();
    let memory = MemoryId::generate();
    let a = EntryId::generate();
    let b = EntryId::generate();
    let append = |seq, entry, text: &str| {
        event(
            seq,
            EventPayload::MemoryContentAppended {
                id: memory,
                entry_id: entry,
                asserted_at: Timestamp::from_millis(1),
                occurred_at: None,
                text: text.to_owned(),
                told_by: Teller::Agent,
                told_in: None,
                visibility: Visibility::Public,
            },
        )
    };
    let arbitrate = |seq, credited: Vec<EntryId>| {
        event(
            seq,
            EventPayload::belief_arbitrated(
                memory,
                vec![a, b],
                ArbitrationResolution {
                    credited,
                    statement: "one says auditorium, another rooftop".to_owned(),
                },
                None,
            ),
        )
    };
    graph
        .apply(&event(
            1,
            EventPayload::memory_created(memory, Namespace::Event.with_name("all-hands")),
        ))
        .unwrap();
    graph.apply(&append(2, a, "in the auditorium")).unwrap();
    graph.apply(&append(3, b, "on the rooftop")).unwrap();

    // Unresolved: both competing entries are disputed.
    graph.apply(&arbitrate(4, vec![])).unwrap();
    assert_eq!(
        graph.disputed_entries(memory).unwrap(),
        [a, b].into_iter().collect()
    );

    // Crediting a side settles it: nothing disputed.
    graph.apply(&arbitrate(5, vec![a])).unwrap();
    assert!(graph.disputed_entries(memory).unwrap().is_empty());

    // Back to unresolved, then supersede one account — one live competitor is not a dispute.
    graph.apply(&arbitrate(6, vec![])).unwrap();
    let c = EntryId::generate();
    graph
        .apply(&append(7, c, "confirmed: the rooftop"))
        .unwrap();
    graph
        .apply(&event(8, EventPayload::memory_superseded(memory, a, c)))
        .unwrap();
    assert!(graph.disputed_entries(memory).unwrap().is_empty());
}

/// The `occurred_authored` flag distinguishes an occurrence stamped at append (ground truth) from
/// one resolved later by the temporal extraction (inference): an authored append reads back
/// authored, an untimed append resolved by `EntryTemporalResolved` reads back not-authored, and an
/// undated entry is never authored.
#[test]
fn occurred_authored_distinguishes_authored_from_extracted() {
    let mut graph = Graph::open_in_memory().unwrap();
    let memory = MemoryId::generate();
    let authored = EntryId::generate();
    let extracted = EntryId::generate();
    let undated = EntryId::generate();
    let append = |seq, entry, occurred_at| {
        event(
            seq,
            EventPayload::MemoryContentAppended {
                id: memory,
                entry_id: entry,
                asserted_at: Timestamp::from_millis(1),
                occurred_at,
                text: "fact".to_owned(),
                told_by: Teller::Agent,
                told_in: None,
                visibility: Visibility::Public,
            },
        )
    };
    graph
        .apply(&event(
            1,
            EventPayload::memory_created(memory, Namespace::Event.with_name("cutover")),
        ))
        .unwrap();
    graph
        .apply(&append(
            2,
            authored,
            Some(TemporalRef::Day(CivilDate("2026-07-20".into()))),
        ))
        .unwrap();
    graph.apply(&append(3, extracted, None)).unwrap();
    graph.apply(&append(4, undated, None)).unwrap();
    // The extraction pass resolves the untimed entry's occurrence.
    graph
        .apply(&event(
            5,
            EventPayload::entry_temporal_resolved(
                memory,
                extracted,
                TemporalRef::Day(CivilDate("2026-06-08".into())),
                None,
            ),
        ))
        .unwrap();

    let authored_of = |entry_id: EntryId| {
        graph
            .entry_by_id(entry_id)
            .unwrap()
            .expect("the entry projects")
            .1
            .occurred_authored
    };
    assert!(authored_of(authored), "an authored append is ground truth");
    assert!(
        !authored_of(extracted),
        "an extracted occurrence is inference, not authored"
    );
    assert!(
        !authored_of(undated),
        "an undated entry has no occurrence to classify"
    );
}

/// A weekly recurring memory surfaces in `recurring_in_window` when its next virtual instance falls
/// in the window — the calendar.upcoming expansion path. Reproduces the standup case: asserted on a
/// Monday, queried the following week, the next instance a few days out must be found.
#[test]
fn recurring_in_window_surfaces_a_weekly_instance() {
    let mut graph = Graph::open_in_memory().unwrap();
    let memory = MemoryId::generate();
    let entry = EntryId::generate();
    let asserted = Timestamp::from_millis(1_780_876_810_000); // 2026-06-08T00:00:10 (a Monday).
    graph
        .apply(&event(
            1,
            EventPayload::memory_created(memory, Namespace::Event.with_name("standup")),
        ))
        .unwrap();
    graph
        .apply(&event(
            2,
            EventPayload::MemoryContentAppended {
                id: memory,
                entry_id: entry,
                asserted_at: asserted,
                occurred_at: Some(TemporalRef::Recurring(Rrule("FREQ=WEEKLY;BYDAY=MO".into()))),
                text: "Recurring every Monday".to_owned(),
                told_by: Teller::Agent,
                told_in: None,
                visibility: Visibility::Public,
            },
        ))
        .unwrap();

    // Query the following week — the next instance (2026-06-22) is ~6 days into the 7-day window.
    let from = Timestamp::from_millis(1_781_568_034_855); // 2026-06-16T00:00:34.
    let to = Timestamp::from_millis(from.as_millis() + 7 * MILLIS_PER_DAY);
    let hits = graph.recurring_in_window(from, to).unwrap();
    assert_eq!(
        hits.len(),
        1,
        "the weekly standup should surface in the upcoming window, got {hits:?}"
    );
}
