//! Attributed cross-teller merge scenario: tier-1 consolidation merges two `Attributed` near-duplicates
//! from different tellers into one agent-founded replacement — and must carry each source teller onto it
//! as an attestation rather than laundering their accounts into the agent. Attribution is the property
//! under test: after the merge, neither teller's standing may be lost.
//!
//! Two `Attributed` accounts of the same fact are seeded on one person, told by Erin and Frank. Because
//! both surface to everyone, tier 1 is permitted to merge them across tellers — the synthesized
//! replacement founds under `Teller::Agent`, and each real teller rides as an `EntryAttested` carrying
//! their account forward (rendered later as a `[via Erin, Frank]` provenance marker). The gate credits
//! attribution as preserved whether the merge fired (both tellers survive as attestations on the live
//! replacement) or did not (both original accounts are still live). It fails only on genuine attribution
//! *loss* — a merge that dropped a teller's account without carrying it onto the replacement.
//!
//! The two accounts are one word apart ("grad" vs "graduate"), the rest identical, so their contextual
//! cosine sits well above `consolidation_similarity_threshold` and they cluster robustly — near-identical
//! is the safe band for making the tier-1 merge actually fire (`zuihitsu debug embed` is the tuning
//! tool). Whether it fires is a reported metric, since the band is embedder-dependent; the attribution
//! gate holds under either outcome.

use std::{collections::BTreeSet, sync::Arc};

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

/// This module's scenarios.
pub fn scenarios() -> Vec<Arc<dyn Scenario>> {
    vec![Arc::new(AttributedMergePreservesAttribution)]
}

/// Erin's and Frank's near-identical attributed accounts of the same fact — one word apart, so they
/// cluster tightly at the consolidation bar.
const ERIN_ACCOUNT: &str = "Rowan mentored the new grad cohort last quarter.";
const FRANK_ACCOUNT: &str = "Rowan mentored the new graduate cohort last quarter.";

/// Seeds two cross-teller `Attributed` near-duplicates on `person/rowan` and drives a maintenance pass.
/// The gate is that neither teller's attribution is lost; the metric is that the merge fired.
pub struct AttributedMergePreservesAttribution;

#[async_trait]
impl Scenario for AttributedMergePreservesAttribution {
    fn meta(&self) -> ScenarioMeta {
        ScenarioMeta {
            name: "attributed_merge_preserves_attribution".to_owned(),
            category: Category::Writes,
            description: "Two Attributed near-duplicate accounts of one fact, told by different \
                          tellers, are eligible for a tier-1 cross-teller merge. When the merge fires, \
                          the agent-founded replacement must carry each source teller as an attestation \
                          rather than collapsing their accounts into the agent. The gate is that neither \
                          teller's attribution is lost — both survive as attestations on the replacement, \
                          or both originals stay live if no merge fired; it fails only on attribution \
                          loss. The merge firing is a reported metric."
                .to_owned(),
            bar: Bar::gating(),
        }
    }

    fn steps(&self) -> Vec<EvalStep> {
        let rowan = MemoryId::generate();
        let erin = MemoryId::generate();
        let frank = MemoryId::generate();
        let now = run_start();
        let seed = vec![
            EventPayload::memory_created(rowan, MemoryName::new("person/rowan")),
            EventPayload::memory_created(erin, MemoryName::new("person/erin")),
            EventPayload::memory_created(frank, MemoryName::new("person/frank")),
            append(rowan, now, ERIN_ACCOUNT, Teller::Participant(erin)),
            append(rowan, now, FRANK_ACCOUNT, Teller::Participant(frank)),
        ];
        vec![
            EvalStep::SeedEvents(seed),
            EvalStep::Settle,
            EvalStep::MaintenanceCatchUp,
            EvalStep::Settle,
        ]
    }

    async fn assess(&self, events: &[Event], _judge: &Judge) -> Vec<Verdict> {
        let entries = analysis::entries(events);
        let hidden: BTreeSet<EntryId> = analysis::superseded_entry_ids(events)
            .union(&analysis::retracted_entry_ids(events))
            .copied()
            .collect();
        let live_ids: BTreeSet<EntryId> = entries
            .iter()
            .filter(|entry| !hidden.contains(&entry.entry_id))
            .map(|entry| entry.entry_id)
            .collect();

        let erin = analysis::memory_id_named(events, "person/erin");
        let frank = analysis::memory_id_named(events, "person/frank");
        let e_erin = entry_id_of(&entries, ERIN_ACCOUNT);
        let e_frank = entry_id_of(&entries, FRANK_ACCOUNT);
        let attestations = analysis::attestations(events);

        // A teller's account is represented iff its original is still live, or it survives as an
        // attestation on a live entry (the replacement a merge founded). Attribution is lost only when
        // neither holds — a merge dropped the teller without carrying their account forward.
        let represented = |teller: Option<MemoryId>, original: Option<EntryId>| -> bool {
            let original_live = original.is_some_and(|id| live_ids.contains(&id));
            let attested_live = teller.is_some_and(|teller| {
                attestations.iter().any(|attestation| {
                    attestation.teller == Teller::Participant(teller)
                        && live_ids.contains(&attestation.entry)
                })
            });
            original_live || attested_live
        };
        let erin_kept = represented(erin, e_erin);
        let frank_kept = represented(frank, e_frank);

        // Metric: the tier-1 merge fired — one `EntriesConsolidated` whose sources are both accounts,
        // producing a single replacement. Band-dependent, so reported.
        let merged = match (e_erin, e_frank) {
            (Some(erin_entry), Some(frank_entry)) => events.iter().any(|event| {
                matches!(
                    &event.payload,
                    EventPayload::EntriesConsolidated { sources, .. }
                        if sources.contains(&erin_entry) && sources.contains(&frank_entry)
                )
            }),
            _ => false,
        };

        vec![
            Verdict::oracle_outcome(
                "preserved both tellers' attribution across the merge",
                erin_kept && frank_kept,
                "neither teller's account was lost — each is still live or carried onto the \
                 replacement as an attestation",
                "attribution was lost: a merge dropped a teller's account without carrying it onto the \
                 replacement as an attestation",
            ),
            Verdict::metric_outcome(
                "merged the two attributed accounts into one replacement",
                merged,
                "the two attributed near-duplicates were consolidated into a single replacement",
                "the two attributed accounts were not merged (they did not clear the consolidation bar)",
            ),
        ]
    }
}

/// The entry id of the (first) entry whose text exactly matches `text`, live or not.
fn entry_id_of(entries: &[analysis::EntryFacts], text: &str) -> Option<EntryId> {
    let text = text.trim().to_lowercase();
    entries
        .iter()
        .find(|entry| entry.text.trim().to_lowercase() == text)
        .map(|entry| entry.entry_id)
}

fn append(id: MemoryId, now: Timestamp, text: &str, told_by: Teller) -> EventPayload {
    EventPayload::MemoryContentAppended {
        id,
        entry_id: EntryId::generate(),
        asserted_at: now,
        occurred_at: None,
        text: text.to_owned(),
        told_by,
        told_in: None,
        visibility: Visibility::Attributed,
    }
}
