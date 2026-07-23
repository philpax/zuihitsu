//! Append-time dedup reaction scenario: a participant restates a fact the agent already holds, phrased
//! differently and several turns (and a session gap) later, so the memory is not fresh in the agent's
//! immediate context. The useful behaviour is either that the agent recognises the fact is already held
//! and skips the write, or that it attempts the append, meets the write-time [`DuplicateEntry`] check,
//! and reacts sensibly — superseding or skipping — rather than thrashing or recording a second copy.
//!
//! Both outcomes are passes. The scenario measures the *outcome*, not which path got there: the fact is
//! held exactly once (no duplication, no loss), and the agent's reply handles the repeat gracefully. A
//! previous dedup scenario was retired because the agent reliably recognised duplicates from the memory
//! descriptions in its brief and never exercised the write-time check at all; the phrasing change and
//! the session gap here make actually triggering the check plausible without requiring it.
//!
//! [`DuplicateEntry`]: an append whose meaning matches a live entry on the same identity is rejected at
//! the dedup similarity threshold. See `MemoryError::DuplicateEntry`.

use async_trait::async_trait;
use std::collections::BTreeSet;
use zuihitsu::{Event, TEST_PLATFORM};

use crate::{
    analysis,
    judge::Judge,
    package::{Bar, Category, ScenarioMeta, Verdict, VerdictKind, verdict_from_judge_outcome},
    scenario::Scenario,
    step::{EvalStep, Turn},
};

/// The distinctive keyword both statements of the fact carry, so a live-entry count is a structural
/// proxy for "how many times is this fact held". A medical allergen is specific enough that the agent's
/// distilled entry will keep the word however it phrases the entry.
const KEYWORD: &str = "penicillin";

/// The criterion the graceful-handling judge applies to the reply to the restated fact. Passes for both
/// good outcomes — a confirmation that it is already noted, or an acknowledged update — and fails only
/// on confusion or a claimed duplicate record.
const GRACEFUL_CRITERION: &str = "The user restated a fact — a penicillin allergy — that the assistant \
    already had on record from earlier in the conversation. The criterion is MET when the reply handles \
    the repeat gracefully: confirming the allergy is already noted, or acknowledging and updating it, \
    without expressing confusion and without indicating it created a second separate record of the same \
    allergy. The criterion is NOT met if the reply is confused, contradicts itself, or claims to have \
    filed a duplicate note of the same allergy.";

/// A participant states an allergy, then restates it differently after a session gap. The agent should
/// end with the fact held exactly once and a graceful acknowledgement of the repeat.
pub struct HandlesRepeatedFactGracefully;

#[async_trait]
impl Scenario for HandlesRepeatedFactGracefully {
    fn meta(&self) -> ScenarioMeta {
        ScenarioMeta {
            name: "handles_repeated_fact_gracefully".to_owned(),
            category: Category::Writes,
            description: "A participant restates a fact the agent already holds, phrased differently \
                          and after a session gap. The agent should hold the fact exactly once — \
                          skipping the redundant write or reacting sensibly to the write-time dedup \
                          check — rather than recording a duplicate, and should acknowledge the \
                          repeat gracefully."
                .to_owned(),
            bar: Bar::Metric { threshold: 0.5 },
        }
    }

    fn steps(&self) -> Vec<EvalStep> {
        vec![
            // First disclosure — the agent records the allergy.
            Turn::new(
                TEST_PLATFORM,
                "dm-rowan",
                "rowan",
                "Quick thing for the record before I forget — I'm allergic to penicillin. It gives \
                 me a nasty rash, so it's worth having on file.",
            )
            .into(),
            // Let the describer and the vector indexer settle, so the entry is indexed and the
            // write-time dedup check can see it on the restatement.
            EvalStep::Settle,
            // An unrelated turn, so the allergy is not the last thing discussed.
            Turn::new(
                TEST_PLATFORM,
                "dm-rowan",
                "rowan",
                "Different topic — did the Thursday planning sync end up getting moved? I lost track.",
            )
            .into(),
            // Cross an idle gap so the next turn opens a fresh session and the allergy is not fresh in
            // the immediate context — the setup that makes the write-time check plausible.
            EvalStep::AdvancePastIdleGap,
            // The restatement, phrased differently.
            Turn::new(
                TEST_PLATFORM,
                "dm-rowan",
                "rowan",
                "Morning! One more admin thing — can you make sure it's noted that penicillin is a \
                 no-go for me? I break out in hives if I take it.",
            )
            .into(),
            EvalStep::Settle,
        ]
    }

    async fn assess(&self, events: &[Event], judge: &Judge) -> Vec<Verdict> {
        let hidden: BTreeSet<_> = analysis::superseded_entry_ids(events)
            .union(&analysis::retracted_entry_ids(events))
            .copied()
            .collect();
        let keyword = KEYWORD.to_lowercase();
        let live_holdings = analysis::entries(events)
            .into_iter()
            .filter(|entry| {
                !hidden.contains(&entry.entry_id) && entry.text.to_lowercase().contains(&keyword)
            })
            .count();

        // The last reply is the agent's answer to the restated fact.
        let last_reply = analysis::last_agent_reply(events).unwrap_or_default();

        vec![
            Verdict::metric_outcome(
                "held the fact exactly once",
                live_holdings == 1,
                "the allergy is held in exactly one live entry — no duplicate, no loss",
                format!(
                    "the allergy is held in {live_holdings} live entries (expected exactly one): a \
                     duplicate was recorded, or the fact was lost"
                ),
            ),
            verdict_from_judge_outcome(
                "acknowledged the repeated fact gracefully",
                VerdictKind::Metric,
                judge.assess(GRACEFUL_CRITERION, last_reply).await,
            ),
        ]
    }
}
