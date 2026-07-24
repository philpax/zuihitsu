//! The shared activity-proportional scheduling gate for maintenance passes.

use crate::{engine::Engine, ids::Seq, instance::InstanceError};

/// Whether enough activity has accrued since `last_cursor` to justify running a pass. Compares the
/// event log head against `last_cursor` and returns true when the gap meets `min_activity`. A
/// `min_activity` of 0 disables the gate (always runs). This counts all events since the cursor,
/// not just content-entry appends — a heuristic that is simpler and still effective, since a busy
/// instance with many events almost certainly has content changes among them. The settings field
/// names (`consolidation_min_activity`, etc.) reflect this: they are event-count thresholds, not
/// content-change counts.
pub fn activity_gate(
    engine: &Engine,
    last_cursor: Seq,
    min_activity: i64,
) -> Result<bool, InstanceError> {
    if min_activity <= 0 {
        return Ok(true);
    }
    let head = engine.store.lock().head()?;
    Ok(i64::try_from(head.0.saturating_sub(last_cursor.0)).unwrap_or(i64::MAX) >= min_activity)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        clock::ManualClock,
        engine::Engine,
        event::{EventPayload, EventSource},
        graph::Graph,
        store::{MemoryStore, Store},
        time::Timestamp,
    };
    use std::sync::Arc;

    fn engine_with_events(count: usize) -> Arc<Engine> {
        let mut store = MemoryStore::new();
        let payloads: Vec<EventPayload> = (0..count)
            .map(|_| {
                EventPayload::memory_created(
                    crate::ids::MemoryId::generate(),
                    crate::ids::Namespace::Person.with_name("test"),
                )
            })
            .collect();
        store
            .append(Timestamp::from_millis(1), EventSource::Agent, payloads)
            .unwrap();
        let graph = Graph::open_in_memory().unwrap();
        Engine::new(
            Box::new(store),
            graph,
            Box::new(ManualClock::new(Timestamp::from_millis(1))),
        )
    }

    #[test]
    fn activity_gate_blocks_below_threshold_and_allows_above() {
        let engine = engine_with_events(10);
        let head = engine.store.lock().head().unwrap();

        // Below the threshold: gate blocks.
        assert!(!activity_gate(&engine, head, 5).unwrap());
        // At the threshold (head is 10, cursor at 0, gap is 10 >= 5): gate allows.
        assert!(activity_gate(&engine, Seq::ZERO, 5).unwrap());
        // Zero threshold disables the gate.
        assert!(activity_gate(&engine, head, 0).unwrap());
    }
}
