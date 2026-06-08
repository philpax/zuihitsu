//! Scheduled work: the fire and drain halves of the wake-up lifecycle (spec §Scheduled work,
//! §Agent-initiated speech).
//!
//! [`fire_due`] is the scheduler — it derives wake-ups from the calendar (entries scheduled for the
//! future whose time has now passed) and records `ScheduledJobFired`, pinning each firing into the log
//! so the surface is a function of the log rather than a live clock. It is global and
//! conversation-agnostic: a later background driver (deferred — see the Stage 10 roadmap) runs it on a
//! timer, while for now the server calls it as a catch-up when a session opens. [`drain`] is the
//! delivery half — it reads the fired-but-unsurfaced surface, keeps the items that are both visible to
//! and targeted at the present set, and formats them for the server to raise as an `Initiated` system
//! turn (recording `ScheduledItemSurfaced` so each is raised once).

use crate::{
    event::EventPayload,
    graph::{EntryView, Graph, GraphError, MemoryView},
    ids::{EntryId, MemoryId},
    settings::SchedulerSettings,
    store::{Store, StoreError},
    time::{self, Timestamp},
};

use super::visibility;

/// A scheduled-work failure, delegating to the store or graph beneath it.
#[derive(Debug)]
pub enum SchedulerError {
    Store(StoreError),
    Graph(GraphError),
}

impl std::fmt::Display for SchedulerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SchedulerError::Store(error) => write!(f, "scheduler (store): {error}"),
            SchedulerError::Graph(error) => write!(f, "scheduler (graph): {error}"),
        }
    }
}

impl std::error::Error for SchedulerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SchedulerError::Store(error) => Some(error),
            SchedulerError::Graph(error) => Some(error),
        }
    }
}

impl From<StoreError> for SchedulerError {
    fn from(error: StoreError) -> SchedulerError {
        SchedulerError::Store(error)
    }
}

impl From<GraphError> for SchedulerError {
    fn from(error: GraphError) -> SchedulerError {
        SchedulerError::Graph(error)
    }
}

/// Fire every wake-up that has come due by `now`: append a `ScheduledJobFired` for each entry the
/// comes-due rule selects (see [`Graph::due_occurrences`]). Global — it fires across all conversations,
/// not just an opening one — and the single point a live clock enters the surface, made replayable by
/// the events it writes. Returns the number fired; the caller materializes so the firings are visible
/// to the drain.
pub fn fire_due(
    store: &mut dyn Store,
    graph: &Graph,
    now: Timestamp,
) -> Result<usize, SchedulerError> {
    let due = graph.due_occurrences(now)?;
    if due.is_empty() {
        return Ok(0);
    }
    let payloads = due
        .into_iter()
        .map(|(memory, entry_id)| EventPayload::ScheduledJobFired {
            entry_id,
            memory,
            fired_at: now,
        })
        .collect();
    Ok(store.append(now, payloads)?.len())
}

/// The content a session-open drain delivers: the formatted block to raise and the entries it covers
/// (each as `(entry, memory)`), so the caller can record a `ScheduledItemSurfaced` per entry.
pub struct Drained {
    pub text: String,
    pub entries: Vec<(EntryId, MemoryId)>,
}

/// Select the fired wake-ups eligible for a session with present set `present_set` — those both
/// `visible(...)` to and targeted at the present set (spec §Agent-initiated speech) — up to
/// `settings.max_wakeups_per_session`, and format them. Returns `None` when nothing is eligible. Pure:
/// the caller appends the system turn and the `ScheduledItemSurfaced` markers.
pub fn drain(
    graph: &Graph,
    present_set: &[MemoryId],
    settings: &SchedulerSettings,
) -> Result<Option<Drained>, SchedulerError> {
    let cap = settings.max_wakeups_per_session.max(0) as usize;
    if cap == 0 {
        return Ok(None);
    }
    let class_of = |id| graph.class_id(id).map(|class| class.unwrap_or(id));
    let mut lines = Vec::new();
    let mut entries = Vec::new();
    for (memory, entry) in graph.pending_wakeups()? {
        if entries.len() >= cap {
            break;
        }
        if !visibility::visible(&entry, &memory, present_set, &class_of)?
            || !visibility::targets_present(&entry, &memory, present_set, &class_of)?
        {
            continue;
        }
        lines.push(format_wakeup(&memory, &entry));
        entries.push((entry.entry_id, memory.id));
    }
    if entries.is_empty() {
        return Ok(None);
    }
    Ok(Some(Drained {
        text: format!("# Due now\n{}", lines.join("\n")),
        entries,
    }))
}

/// One wake-up line: the entry's text, its memory's handle, and the occurrence date when known.
fn format_wakeup(memory: &MemoryView, entry: &EntryView) -> String {
    match entry.occurred_sort {
        Some(at) => format!(
            "- {} ({}), {}",
            entry.text,
            memory.name.as_str(),
            time::format_day(at)
        ),
        None => format!("- {} ({})", entry.text, memory.name.as_str()),
    }
}

#[cfg(test)]
mod tests {
    use super::{drain, fire_due};
    use crate::{
        event::{EventPayload, Teller, Visibility},
        graph::Graph,
        ids::{EntryId, MemoryId, MemoryName, Seq},
        settings::SchedulerSettings,
        store::{MemoryStore, Store},
        time::{Rrule, TemporalRef, Timestamp},
    };

    /// A store + graph materialized from `payloads`, committed at `at`.
    fn world(at: i64, payloads: Vec<EventPayload>) -> (MemoryStore, Graph) {
        let mut store = MemoryStore::new();
        store.append(Timestamp::from_millis(at), payloads).unwrap();
        let mut graph = Graph::open_in_memory().unwrap();
        graph.materialize_from(&store).unwrap();
        (store, graph)
    }

    fn created(id: MemoryId, name: &str) -> EventPayload {
        EventPayload::MemoryCreated {
            id,
            name: MemoryName::new(name),
        }
    }

    /// A content entry asserted at `asserted_ms` whose occurrence is `occurred_at`, told by `told_by`.
    fn appended(
        id: MemoryId,
        entry_id: EntryId,
        asserted_ms: i64,
        occurred_at: Option<TemporalRef>,
        told_by: Teller,
        visibility: Visibility,
    ) -> EventPayload {
        EventPayload::MemoryContentAppended {
            id,
            entry_id,
            asserted_at: Timestamp::from_millis(asserted_ms),
            occurred_at,
            text: "the thing".to_owned(),
            told_by,
            told_in: None,
            visibility,
        }
    }

    fn instant(ms: i64) -> Option<TemporalRef> {
        Some(TemporalRef::Instant(Timestamp::from_millis(ms)))
    }

    /// The `entry_id`s of the `ScheduledJobFired` events in the store, in order.
    fn fired(store: &MemoryStore) -> Vec<EntryId> {
        store
            .read_from(Seq::ZERO)
            .unwrap()
            .into_iter()
            .filter_map(|e| match e.payload {
                EventPayload::ScheduledJobFired { entry_id, .. } => Some(entry_id),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn fires_a_future_scheduled_entry_once_due_and_is_idempotent() {
        // Asserted at 1_000, scheduled for 5_000: at now=6_000 it has come due.
        let id = MemoryId::generate();
        let entry = EntryId::generate();
        let (mut store, mut graph) = world(
            1_000,
            vec![
                created(id, "event/dentist"),
                appended(
                    id,
                    entry,
                    1_000,
                    instant(5_000),
                    Teller::Agent,
                    Visibility::Public,
                ),
            ],
        );

        assert_eq!(
            fire_due(&mut store, &graph, Timestamp::from_millis(6_000)).unwrap(),
            1
        );
        graph.materialize_from(&store).unwrap();
        assert_eq!(fired(&store), vec![entry]);

        // A second pass fires nothing — the entry is already fired.
        assert_eq!(
            fire_due(&mut store, &graph, Timestamp::from_millis(7_000)).unwrap(),
            0
        );
        graph.materialize_from(&store).unwrap();
        assert_eq!(fired(&store), vec![entry]);
    }

    #[test]
    fn ignores_past_anchored_future_and_recurring_occurrences() {
        let past = MemoryId::generate();
        let future = MemoryId::generate();
        let recurring = MemoryId::generate();
        let (mut store, graph) = world(
            5_000,
            vec![
                // Recorded at 5_000 about an event at 1_000 — a past event, never a wake-up.
                created(past, "event/last-week"),
                appended(
                    past,
                    EntryId::generate(),
                    5_000,
                    instant(1_000),
                    Teller::Agent,
                    Visibility::Public,
                ),
                // Scheduled for 10_000 — still in the future at now=6_000.
                created(future, "event/next-week"),
                appended(
                    future,
                    EntryId::generate(),
                    5_000,
                    instant(10_000),
                    Teller::Agent,
                    Visibility::Public,
                ),
                // Recurring — null occurred_sort, never fires here.
                created(recurring, "event/standup"),
                appended(
                    recurring,
                    EntryId::generate(),
                    5_000,
                    Some(TemporalRef::Recurring(Rrule("FREQ=WEEKLY".into()))),
                    Teller::Agent,
                    Visibility::Public,
                ),
            ],
        );

        assert_eq!(
            fire_due(&mut store, &graph, Timestamp::from_millis(6_000)).unwrap(),
            0
        );
    }

    #[test]
    fn drain_delivers_only_visible_and_targeted_items() {
        let phil = MemoryId::generate();
        let erin = MemoryId::generate();
        let dentist = MemoryId::generate();
        let entry = EntryId::generate();
        // A public reminder on event/dentist, told by Phil → target = {Phil}.
        let (mut store, mut graph) = world(
            1_000,
            vec![
                created(phil, "person/phil"),
                created(erin, "person/erin"),
                created(dentist, "event/dentist"),
                appended(
                    dentist,
                    entry,
                    1_000,
                    instant(5_000),
                    Teller::Participant(phil),
                    Visibility::Public,
                ),
            ],
        );
        fire_due(&mut store, &graph, Timestamp::from_millis(6_000)).unwrap();
        graph.materialize_from(&store).unwrap();
        let settings = SchedulerSettings::default();

        // Phil present: delivered, naming the memory and date.
        let drained = drain(&graph, &[phil], &settings)
            .unwrap()
            .expect("eligible for Phil");
        assert_eq!(drained.entries, vec![(entry, dentist)]);
        assert!(drained.text.contains("# Due now"));
        assert!(drained.text.contains("event/dentist"));

        // Only Erin present: Phil is the target and isn't here, so nothing drains.
        assert!(drain(&graph, &[erin], &settings).unwrap().is_none());
    }

    #[test]
    fn drain_respects_the_visibility_predicate() {
        // A private aside about Phil, told by Erin, scheduled in the future.
        let phil = MemoryId::generate();
        let erin = MemoryId::generate();
        let entry = EntryId::generate();
        let (mut store, mut graph) = world(
            1_000,
            vec![
                created(phil, "person/phil"),
                created(erin, "person/erin"),
                appended(
                    phil,
                    entry,
                    1_000,
                    instant(5_000),
                    Teller::Participant(erin),
                    Visibility::PrivateToTeller,
                ),
            ],
        );
        fire_due(&mut store, &graph, Timestamp::from_millis(6_000)).unwrap();
        graph.materialize_from(&store).unwrap();
        let settings = SchedulerSettings::default();

        // Erin alone: visible (teller present) and targeted (subject Phil), so it drains to her.
        assert!(drain(&graph, &[erin], &settings).unwrap().is_some());
        // Phil present too: the subject-guard suppresses the aside — not visible, so withheld.
        assert!(drain(&graph, &[erin, phil], &settings).unwrap().is_none());
    }

    #[test]
    fn drain_caps_the_number_of_items() {
        let phil = MemoryId::generate();
        let mut payloads = vec![created(phil, "person/phil")];
        for i in 0..4 {
            let id = MemoryId::generate();
            payloads.push(created(id, &format!("event/e{i}")));
            payloads.push(appended(
                id,
                EntryId::generate(),
                1_000,
                instant(5_000 + i),
                Teller::Participant(phil),
                Visibility::Public,
            ));
        }
        let (mut store, mut graph) = world(1_000, payloads);
        fire_due(&mut store, &graph, Timestamp::from_millis(6_000)).unwrap();
        graph.materialize_from(&store).unwrap();

        let settings = SchedulerSettings {
            max_wakeups_per_session: 2,
        };
        let drained = drain(&graph, &[phil], &settings)
            .unwrap()
            .expect("eligible");
        assert_eq!(drained.entries.len(), 2);
    }
}
