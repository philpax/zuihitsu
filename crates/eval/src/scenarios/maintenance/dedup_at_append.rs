//! Append-time dedup scenario: the agent attempts to append a fact that semantically duplicates
//! a live entry. The gating oracle checks that the agent's deliberation surfaces a teachable
//! error and does not re-append.

use async_trait::async_trait;
use zuihitsu::{
    EntryId, Event, EventPayload, MemoryId, MemoryName, TEST_PLATFORM, Teller, Visibility,
};

use crate::{
    analysis,
    context::run_start,
    judge::Judge,
    package::{Bar, Category, ScenarioMeta, Verdict, VerdictKind, verdict_from_judge_outcome},
    scenario::Scenario,
    step::{EvalStep, Turn},
};

/// A person already has a live entry "is a senior developer" (no name — the handle carries it).
/// A participant tells the agent "Rowan is a senior developer" — semantically near-identical once
/// the name is stripped. The gating oracle checks that the agent does not blindly re-append the
/// duplicate, and either skips the write or surfaces the teachable error.
pub struct RejectsSemanticDuplicate;

#[async_trait]
impl Scenario for RejectsSemanticDuplicate {
    fn meta(&self) -> ScenarioMeta {
        ScenarioMeta {
            name: "rejects_semantic_duplicate".to_owned(),
            category: Category::Writes,
            description: "A person already holds a live entry. A participant tells the agent a \
                          semantically near-identical fact. The agent should not blindly re-append \
                          the duplicate — it should either skip the write or surface the teachable \
                          error from the dedup check."
                .to_owned(),
            bar: Bar::gating(),
        }
    }

    fn steps(&self) -> Vec<EvalStep> {
        let rowan = MemoryId::generate();
        let now = run_start();
        let seed = vec![
            EventPayload::memory_created(rowan, MemoryName::new("person/rowan")),
            EventPayload::MemoryContentAppended {
                id: rowan,
                entry_id: EntryId::generate(),
                asserted_at: now,
                occurred_at: None,
                text: "is a senior developer".to_owned(),
                told_by: Teller::Agent,
                told_in: None,
                visibility: Visibility::Public,
            },
        ];
        vec![
            EvalStep::SeedEvents(seed),
            EvalStep::Settle,
            Turn::new(
                TEST_PLATFORM,
                "dev",
                "casey",
                "Hey, I wanted to make sure you have this down — Rowan is a senior developer. \
                 Can you confirm that's recorded?",
            )
            .into(),
            EvalStep::Settle,
        ]
    }

    async fn assess(&self, events: &[Event], judge: &Judge) -> Vec<Verdict> {
        // Structural: the seeded entry "is a senior developer" should not appear as a second live
        // entry. The dedup check should prevent a near-identical duplicate from landing. The
        // participant's message restates the existing fact, so the agent should either skip the
        // write (the fact is already held) or have the dedup check reject the duplicate append.
        let duplicate_count = analysis::entries(events)
            .iter()
            .filter(|e| {
                e.memory.to_lowercase().contains("rowan")
                    && e.text.to_lowercase().contains("senior developer")
            })
            .count();
        let at_most_one = duplicate_count <= 1;

        // The reply should not claim a write that didn't happen, or should acknowledge the
        // existing entry rather than re-recording it.
        let reply = analysis::last_agent_reply(events).unwrap_or_default();
        let evidence = format!(
            "A participant asked the assistant to confirm that \"Rowan is a senior developer\" is \
             recorded, but the assistant already held that exact entry. The assistant replied:\n\
             \"{reply}\""
        );
        let acknowledged = judge
            .assess(
                "The reply does not claim to have recorded a new fact that was already held. It \
                 either acknowledges the existing entry, notes the duplication, or simply confirms \
                 the known fact without claiming a fresh write. A reply that claims to have just \
                 recorded or saved the fact does not count.",
                &evidence,
            )
            .await;

        vec![
            Verdict::oracle_outcome(
                "left at most one live entry for the duplicate fact",
                at_most_one,
                "the dedup check prevented a second near-identical entry",
                "multiple live entries for the same fact — the dedup check did not fire",
            ),
            verdict_from_judge_outcome(
                "did not claim a write that duplicated an existing entry",
                VerdictKind::Oracle,
                acknowledged,
            ),
        ]
    }
}
