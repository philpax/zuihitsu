//! Consolidation privacy scenario: the consolidation pass must never let a private confidence's
//! detail cross into a wider-audience entry, and must never collapse two differently-attributed
//! accounts into one. Three shapes are seeded on one person and a maintenance pass is driven:
//!
//! - (a) A `PrivateToTeller` confidence carrying a distinctive sentinel detail, semantically related
//!   to but clearly distinct from a `Public` entry on the same topic (the sentinel adds detail the
//!   public entry lacks). It sits below the stricter dedup bar, so tier 2 must leave it alone and its
//!   sentinel must never surface into a `Public` or `Attributed` entry.
//! - (b) A genuine near-duplicate `PrivateToTeller`/`Public` pair (near-identical phrasing). Tier 2
//!   retires the private copy into the public one — the public entry is the replacement, and no new
//!   text is written, so nothing private is copied up to a wider audience.
//! - (c) Two `Attributed` entries from *different* tellers that are near-duplicates of each other.
//!   Below the public level the teller is the audience-bearing payload, so they group per teller and
//!   must never be co-consolidated — collapsing them would drop one teller's attribution.
//!
//! The gating oracles are the two must-not-surface safety properties: the sentinel never reaches a
//! wider-audience entry, and the two different-teller attributed accounts are never collapsed. Both
//! are embedder-threshold-independent — they fail only on a genuine cross-posture or cross-teller
//! consolidation bug, never on where a cosine score happens to land. The mechanism-completeness checks
//! (the sentinel confidence survived, the near-duplicate was retired into its public counterpart) ride
//! as reported metrics, since over- or under-retiring is a quality regression, not a privacy breach,
//! and the visibility model permits either outcome.
//!
//! The similarity bands are empirical against the configured embedding model (the eval runs against a
//! real embedder, not a test stub); `zuihitsu debug embed <a> <b>` is the tuning tool. The (b) pair is
//! deliberately near-identical (one word apart) so it clears the 0.95 dedup bar robustly, and the (a)
//! pair is clearly related but distinct so it stays under it.

use async_trait::async_trait;
use std::collections::BTreeSet;
use zuihitsu::{EntryId, Event, EventPayload, MemoryId, MemoryName, Teller, Timestamp, Visibility};

use crate::{
    analysis,
    context::run_start,
    judge::Judge,
    package::{Bar, Category, ScenarioMeta, Verdict},
    scenario::Scenario,
    step::EvalStep,
};

/// The sentinel detail carried only by the (a) private confidence — a distinctive phrase that must
/// never appear in a wider-audience entry.
const SENTINEL: &str = "glass hinge";

/// (a) The public marathon entry, and the private confidence that adds the sentinel detail. Same
/// topic, clearly distinct: the private one is a fuller, differently-phrased statement, so it sits
/// below the 0.95 dedup bar and tier 2 leaves it live.
const A_PUBLIC: &str = "Rowan is training for the Melbourne Marathon this spring.";
const A_PRIVATE: &str = "Rowan is training for the Melbourne Marathon, though a recurring knee \
                         problem they privately call their glass hinge could force a late withdrawal.";

/// (b) A near-identical public/private pair (one word apart), so it clears the 0.95 dedup bar and
/// tier 2 retires the private copy into the public one.
const B_PUBLIC: &str = "Rowan works as a senior backend engineer at a fintech startup.";
const B_PRIVATE: &str = "Rowan is a senior backend engineer at a fintech startup.";

/// (c) Two near-duplicate `Attributed` accounts from *different* tellers — grouped per teller, they
/// must never be co-consolidated.
const C_ATTR_ERIN: &str = "Rowan volunteers at the local animal shelter on weekends.";
const C_ATTR_FRANK: &str = "Rowan volunteers at the local animal shelter most weekends.";

/// Seeds six entries on `person/rowan` across the public, private, and attributed postures, then drives
/// a maintenance pass. The gating oracles check that the private sentinel never crosses into a
/// wider-audience entry and that the two different-teller attributed accounts are never collapsed.
pub struct ConsolidationPreservesPrivacy;

#[async_trait]
impl Scenario for ConsolidationPreservesPrivacy {
    fn meta(&self) -> ScenarioMeta {
        ScenarioMeta {
            name: "consolidation_preserves_privacy".to_owned(),
            category: Category::Privacy,
            description: "The consolidation pass must never let a private confidence's detail cross \
                          into a wider-audience entry, and must never collapse two differently-\
                          attributed accounts into one. Seeds a private sentinel confidence, a \
                          genuine private/public near-duplicate, and two different-teller attributed \
                          near-duplicates on one person, then drives a maintenance pass."
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
            // (a) public topic + private confidence adding the sentinel.
            append(rowan, now, A_PUBLIC, Teller::Agent, Visibility::Public),
            append(
                rowan,
                now,
                A_PRIVATE,
                Teller::Participant(erin),
                Visibility::PrivateToTeller,
            ),
            // (b) near-identical public/private pair.
            append(rowan, now, B_PUBLIC, Teller::Agent, Visibility::Public),
            append(
                rowan,
                now,
                B_PRIVATE,
                Teller::Participant(erin),
                Visibility::PrivateToTeller,
            ),
            // (c) two attributed near-duplicates from different tellers.
            append(
                rowan,
                now,
                C_ATTR_ERIN,
                Teller::Participant(erin),
                Visibility::Attributed,
            ),
            append(
                rowan,
                now,
                C_ATTR_FRANK,
                Teller::Participant(frank),
                Visibility::Attributed,
            ),
        ];
        vec![
            EvalStep::SeedEvents(seed),
            EvalStep::Settle,
            EvalStep::MaintenanceCatchUp,
            EvalStep::Settle,
        ]
    }

    async fn assess(&self, events: &[Event], _judge: &Judge) -> Vec<Verdict> {
        let hidden = hidden_entry_ids(events);
        let entries = analysis::entries(events);

        // Gating: the sentinel detail never surfaces in a live all-audience entry. tier 2 writes no
        // new text, so even if the (a) confidence were retired the sentinel could only ever sit in the
        // now-superseded private entry — this fails solely on a genuine cross-posture synthesis bug.
        let sentinel = SENTINEL.to_lowercase();
        let sentinel_leaked = entries.iter().any(|entry| {
            !hidden.contains(&entry.entry_id)
                && matches!(
                    entry.visibility,
                    Visibility::Public | Visibility::Attributed
                )
                && entry.text.to_lowercase().contains(&sentinel)
        });

        // Gating: the two different-teller attributed accounts are both still live — grouping per
        // teller, the pass must never collapse them into one. Embedder-independent: they are in
        // separate tier-1 groups and are never tier-2 sources, so this fails only on a cross-teller
        // consolidation bug.
        let attr_erin_live = live_exact(&entries, &hidden, C_ATTR_ERIN);
        let attr_frank_live = live_exact(&entries, &hidden, C_ATTR_FRANK);
        let attributed_kept = attr_erin_live && attr_frank_live;

        // Metric: the (a) private confidence survived (the sentinel is still held privately, not lost
        // to an over-eager dedup). Band-dependent, so reported rather than gated.
        let sentinel_survived = entries.iter().any(|entry| {
            !hidden.contains(&entry.entry_id)
                && matches!(entry.visibility, Visibility::PrivateToTeller)
                && entry.text.to_lowercase().contains(&sentinel)
        });

        // Metric: the (b) near-duplicate private copy was retired into its public counterpart — the
        // consolidation names the pre-existing public entry as the replacement (so no new text was
        // written) and the public entry is still live. Band-dependent, so reported rather than gated.
        let b_private_id = entry_id_of(&entries, B_PRIVATE);
        let b_public_id = entry_id_of(&entries, B_PUBLIC);
        let b_deduped = match (b_private_id, b_public_id) {
            (Some(private), Some(public)) => {
                let retired_into_public = events.iter().any(|event| {
                    matches!(
                        &event.payload,
                        EventPayload::EntriesConsolidated { sources, replacement, .. }
                            if *replacement == public && sources.contains(&private)
                    )
                });
                retired_into_public && !hidden.contains(&public)
            }
            _ => false,
        };

        vec![
            Verdict::oracle_outcome(
                "kept the private sentinel out of every wider-audience entry",
                !sentinel_leaked,
                "no live Public or Attributed entry carries the private sentinel detail",
                format!(
                    "LEAK: a live all-audience entry carries the private sentinel \"{SENTINEL}\""
                ),
            ),
            Verdict::oracle_outcome(
                "kept the two different-teller attributed accounts distinct",
                attributed_kept,
                "both attributed accounts are still live — neither teller's attribution was dropped",
                "an attributed account attributed to one teller was consolidated away, collapsing \
                 two distinctly-attributed accounts",
            ),
            Verdict::metric_outcome(
                "preserved the private confidence carrying the sentinel",
                sentinel_survived,
                "the sentinel confidence is still held privately",
                "the sentinel confidence is no longer live — an over-eager dedup retired it",
            ),
            Verdict::metric_outcome(
                "retired the near-duplicate private copy into its public counterpart",
                b_deduped,
                "the private near-duplicate was consolidated into the existing public entry with no \
                 new text written",
                "the private near-duplicate was not retired into its public counterpart",
            ),
        ]
    }
}

/// The union of superseded and retracted entry ids — every entry dropped from live surfaces.
fn hidden_entry_ids(events: &[Event]) -> BTreeSet<EntryId> {
    analysis::superseded_entry_ids(events)
        .union(&analysis::retracted_entry_ids(events))
        .copied()
        .collect()
}

/// Whether a live entry's text exactly matches `text` (case-insensitive, trimmed).
fn live_exact(entries: &[analysis::EntryFacts], hidden: &BTreeSet<EntryId>, text: &str) -> bool {
    let text = text.trim().to_lowercase();
    entries
        .iter()
        .any(|entry| !hidden.contains(&entry.entry_id) && entry.text.trim().to_lowercase() == text)
}

/// The entry id of the (first) entry whose text exactly matches `text`, live or not — for tracing a
/// specific seeded entry through a consolidation.
fn entry_id_of(entries: &[analysis::EntryFacts], text: &str) -> Option<EntryId> {
    let text = text.trim().to_lowercase();
    entries
        .iter()
        .find(|entry| entry.text.trim().to_lowercase() == text)
        .map(|entry| entry.entry_id)
}

fn append(
    id: MemoryId,
    now: Timestamp,
    text: &str,
    told_by: Teller,
    visibility: Visibility,
) -> EventPayload {
    EventPayload::MemoryContentAppended {
        id,
        entry_id: EntryId::generate(),
        asserted_at: now,
        occurred_at: None,
        text: text.to_owned(),
        told_by,
        told_in: None,
        visibility,
    }
}
