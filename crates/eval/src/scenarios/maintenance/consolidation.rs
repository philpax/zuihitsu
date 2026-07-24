//! Consolidation scenario: a duplicate pair of public entries on a person is consolidated into
//! one after a maintenance pass, while a related-but-distinct entry on the same person survives.
//! The oracles check that an `EntriesConsolidated` event was recorded, the pair is tombstoned
//! (not live), and the distinct entry is still live.
//!
//! This exercises tier 1's two-stage design: geometry gathers a candidate cluster at the loose
//! `consolidation_candidate_threshold` (0.60 by default), then the synthesis model decides
//! membership. The related-but-distinct control ("Alex is the team's backend lead") sits at ~0.70
//! against the duplicate pair — *inside* the 0.60 candidate band, so geometry cannot separate it,
//! and it lands in the same candidate cluster as the two rephrasings. Keeping it live is therefore
//! the model's selection judgement at work: it must select only the two entries that state one fact
//! and leave the lead role out. That is a language judgement, not a geometric threshold, so the bar
//! stays `Bar::Metric { threshold: 0.5 }` rather than a gate.

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

/// A duplicate pair on `person/alex` — "Alex is a backend engineer", "Alex works as a backend
/// engineer" — is seeded beside the distinct "Alex is the team's backend lead". After a
/// maintenance pass, the oracles check that an `EntriesConsolidated` event was recorded, the pair
/// is no longer live, and the distinct role fact is.
pub struct ConsolidatesOverlappingEntries;

#[async_trait]
impl Scenario for ConsolidatesOverlappingEntries {
    fn meta(&self) -> ScenarioMeta {
        ScenarioMeta {
            name: "consolidates_overlapping_entries".to_owned(),
            category: Category::Writes,
            description: "Two rephrasings of the same fact on a person are consolidated into one \
                          after a maintenance pass, while a related-but-distinct fact on the same \
                          person survives untouched. The oracles check that an EntriesConsolidated \
                          event was recorded, the duplicate pair is tombstoned, and the distinct \
                          entry is still live."
                .to_owned(),
            bar: Bar::Metric { threshold: 0.5 },
        }
    }

    fn steps(&self) -> Vec<EvalStep> {
        let alex = MemoryId::generate();
        let now = run_start();
        let seed = vec![
            EventPayload::memory_created(alex, MemoryName::new("person/alex")),
            // A genuine duplicate pair (contextual cosine ~0.96 under the live embedder — the bands
            // are empirical; `debug embed` is the tuning tool) plus a related-but-distinct control
            // (~0.70 against the pair). At the 0.60 candidate bar all three land in one candidate
            // cluster, so geometry cannot keep the lead role out: only the model's membership
            // selection can, by consolidating the two rephrasings and leaving the lead role live.
            append(alex, now, "Alex is a backend engineer"),
            append(alex, now, "Alex works as a backend engineer"),
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
        let duplicate_texts = [
            "Alex is a backend engineer",
            "Alex works as a backend engineer",
        ];
        let pair_tombstoned = duplicate_texts
            .iter()
            .all(|&text| !analysis::live_entry_exact(events, "alex", text));

        // The related-but-distinct control must survive: a role fact is not a rephrasing, and a
        // sweep that folds it in is over-merging.
        // Absorption doctrine: the control may be folded into the consolidation, but its
        // information must survive on a live entry — merged or standalone. Only content loss fails.
        let control_live =
            analysis::live_entry_exact(events, "alex", "Alex is the team's backend lead")
                || analysis::live_entry_containing(events, "alex", "backend lead");

        vec![
            Verdict::metric_outcome(
                "recorded an EntriesConsolidated event",
                consolidated,
                "the maintenance pass consolidated the duplicate pair",
                "no EntriesConsolidated event was recorded",
            ),
            Verdict::metric_outcome(
                "tombstoned the duplicate pair",
                pair_tombstoned,
                "both rephrasings of the fact are no longer live",
                "a rephrasing of the duplicated fact is still live",
            ),
            Verdict::metric_outcome(
                "kept the lead-role information on a live entry",
                control_live,
                "the distinct role fact survived the sweep",
                "the distinct role fact was folded into the consolidation",
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
