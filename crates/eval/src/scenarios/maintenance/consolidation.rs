//! Consolidation scenario: three overlapping public entries on a person are consolidated
//! into one after a maintenance pass. The gating oracle checks that the source entries are
//! tombstoned (not live) and an `EntriesConsolidated` event was recorded.

use async_trait::async_trait;
use zuihitsu::{EntryId, Event, EventPayload, MemoryId, MemoryName, Teller, Timestamp, Visibility};

use crate::{
    analysis,
    context::run_start,
    judge::Judge,
    package::{Bar, Category, ScenarioMeta, Verdict},
    scenario::Scenario,
    step::EvalStep,
};

/// Three overlapping public entries on `person/alex` — "Alex is a backend engineer", "Alex works
/// on the backend", "Alex is the team's backend lead" — are seeded. After a maintenance pass, the
/// gating oracle checks that an `EntriesConsolidated` event was recorded and the source entries
/// are no longer live.
pub struct ConsolidatesOverlappingEntries;

#[async_trait]
impl Scenario for ConsolidatesOverlappingEntries {
    fn meta(&self) -> ScenarioMeta {
        ScenarioMeta {
            name: "consolidates_overlapping_entries".to_owned(),
            category: Category::Writes,
            description: "Three overlapping public entries on a person are consolidated into one \
                          after a maintenance pass. The gating oracle checks that the source \
                          entries are tombstoned and an EntriesConsolidated event was recorded."
                .to_owned(),
            bar: Bar::Metric { threshold: 0.5 },
        }
    }

    fn steps(&self) -> Vec<EvalStep> {
        let alex = MemoryId::generate();
        let now = run_start();
        let seed = vec![
            EventPayload::memory_created(alex, MemoryName::new("person/alex")),
            append(alex, now, "Alex is a backend engineer"),
            append(alex, now, "Alex works on the backend"),
            append(alex, now, "Alex is the team's backend lead"),
        ];
        vec![
            EvalStep::SeedEvents(seed),
            EvalStep::Settle,
            EvalStep::MaintenanceCatchUp,
            EvalStep::Settle,
        ]
    }

    async fn assess(&self, events: &[Event], _judge: &Judge) -> Vec<Verdict> {
        // Structural: an EntriesConsolidated event was recorded.
        let consolidated = events
            .iter()
            .any(|e| matches!(e.payload, EventPayload::EntriesConsolidated { .. }));

        // Structural: the three source entries are no longer live (tombstoned). Check by exact text
        // match — the replacement entry may contain overlapping words, so a substring check would
        // false-positive on the replacement. Each source's full text must not appear as a live entry.
        let source_texts = [
            "Alex is a backend engineer",
            "Alex works on the backend",
            "Alex is the team's backend lead",
        ];
        let all_tombstoned = source_texts
            .iter()
            .all(|&text| !analysis::live_entry_exact(events, "alex", text));

        vec![
            Verdict::metric_outcome(
                "recorded an EntriesConsolidated event",
                consolidated,
                "the maintenance pass consolidated the overlapping entries",
                "no EntriesConsolidated event was recorded",
            ),
            Verdict::metric_outcome(
                "tombstoned all three source entries",
                all_tombstoned,
                "the source entries are no longer live",
                "one or more source entries are still live",
            ),
        ]
    }
}

fn append(id: MemoryId, now: Timestamp, text: &str) -> EventPayload {
    EventPayload::MemoryContentAppended {
        id,
        entry_id: EntryId::generate(),
        asserted_at: now,
        occurred_at: None,
        text: text.to_owned(),
        told_by: Teller::Agent,
        told_in: None,
        visibility: Visibility::Public,
    }
}
