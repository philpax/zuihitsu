//! The describer's staleness escape: under turn-over-background priority the background describer
//! yields to conversation, so a saturated instance could hold its backlog back indefinitely. The
//! escape releases that pressure — once the oldest pending description ages past the configured
//! horizon, the next sweep escalates to conversation priority (spec §Write path → freshness before a
//! brief). These tests drive the escape *decision* (`describe_should_escalate`) against a manual
//! clock; the priority mechanism the decision selects is covered in `model::priority`.
use crate::{
    Instance,
    clock::ManualClock,
    event::{EventPayload, EventSource},
    ids::{MemoryId, MemoryName},
    time::Timestamp,
};

const SECOND_MS: i64 = 1_000;

/// Append a content change that leaves a fresh memory stale (its `last_content_seq` outruns its
/// `last_described_seq`), stamped at `at`, and materialize so the graph reflects it.
fn stale_memory_at(server: &Instance, at: Timestamp) {
    let mem = MemoryId::generate();
    server
        .engine
        .store
        .lock()
        .append(
            at,
            EventSource::Agent,
            vec![EventPayload::memory_created(
                mem,
                MemoryName::new("topic/orphan"),
            )],
        )
        .unwrap();
    server
        .engine
        .graph
        .lock()
        .materialize_from(server.engine.store.lock().as_ref())
        .unwrap();
}

#[test]
fn an_empty_backlog_never_escalates() {
    let server = Instance::in_memory(Box::new(ManualClock::new(Timestamp::from_millis(
        10_000 * SECOND_MS,
    ))))
    .unwrap();
    // Nothing is stale, so there is no oldest pending description to age out.
    assert!(!server.describe_should_escalate().unwrap());
}

#[test]
fn a_fresh_change_does_not_escalate_but_an_aged_one_does() {
    let clock = ManualClock::new(Timestamp::from_millis(0));
    let server = Instance::in_memory(Box::new(clock.clone())).unwrap();
    stale_memory_at(&server, Timestamp::from_millis(0));

    // Just changed: well inside the default horizon (5 minutes), so the background sweep still yields.
    assert!(!server.describe_should_escalate().unwrap());

    // One second before the horizon still yields; one second past it escalates.
    clock.set(Timestamp::from_millis(299 * SECOND_MS));
    assert!(!server.describe_should_escalate().unwrap());
    clock.set(Timestamp::from_millis(301 * SECOND_MS));
    assert!(server.describe_should_escalate().unwrap());
}

#[test]
fn the_horizon_is_configurable() {
    let clock = ManualClock::new(Timestamp::from_millis(0));
    let server = Instance::in_memory(Box::new(clock.clone())).unwrap();
    stale_memory_at(&server, Timestamp::from_millis(0));

    let mut settings = server.control().settings().unwrap();
    settings.concurrency.describe_staleness_escape_seconds = 60;
    server.control().set_settings(settings).unwrap();

    clock.set(Timestamp::from_millis(59 * SECOND_MS));
    assert!(!server.describe_should_escalate().unwrap());
    clock.set(Timestamp::from_millis(61 * SECOND_MS));
    assert!(server.describe_should_escalate().unwrap());
}

#[test]
fn a_zero_horizon_disables_the_escape() {
    let clock = ManualClock::new(Timestamp::from_millis(0));
    let server = Instance::in_memory(Box::new(clock.clone())).unwrap();
    stale_memory_at(&server, Timestamp::from_millis(0));

    let mut settings = server.control().settings().unwrap();
    settings.concurrency.describe_staleness_escape_seconds = 0;
    server.control().set_settings(settings).unwrap();

    // However long the backlog sits, a disabled escape leaves the describer yielding.
    clock.set(Timestamp::from_millis(1_000_000 * SECOND_MS));
    assert!(!server.describe_should_escalate().unwrap());
}

#[test]
fn oldest_stale_content_seq_tracks_the_backlog() {
    let server =
        Instance::in_memory(Box::new(ManualClock::new(Timestamp::from_millis(0)))).unwrap();
    assert_eq!(
        server
            .engine
            .graph
            .lock()
            .oldest_stale_content_seq()
            .unwrap(),
        None,
        "an empty graph has no stale backlog"
    );
    stale_memory_at(&server, Timestamp::from_millis(0));
    let oldest = server
        .engine
        .graph
        .lock()
        .oldest_stale_content_seq()
        .unwrap()
        .expect("a stale memory has an oldest content watermark");
    // The watermark dates to the change's own stamp through the log.
    assert_eq!(
        server.engine.store.lock().recorded_at(oldest).unwrap(),
        Some(Timestamp::from_millis(0))
    );
}
