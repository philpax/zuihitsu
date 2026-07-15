//! Retraction as correction. A fact the agent recorded earlier landed on the wrong person; a later
//! turn surfaces the mistake ("that's Davina's role, not David's"). `<memory>:supersede` can only
//! replace an entry in place on the same memory, so it cannot move a fact to the right one — the honest
//! fix is to retract the mis-filed entry (with a reason) and re-assert it on the correct memory. The
//! oracles are structural where the property is exact — no live residue on the wrong memory, the fact
//! live on the right one, an auditable retraction recorded — and defer the manner of the reply to the
//! judge.

use std::sync::Arc;

use async_trait::async_trait;
use zuihitsu::{
    EntryId, Event, EventPayload, MemoryId, MemoryName, TEST_PLATFORM, Teller, Timestamp,
    Visibility,
};

use crate::{
    analysis,
    context::RUN_START_MS,
    judge::Judge,
    package::{Bar, Category, ScenarioMeta, Verdict, VerdictKind, verdict_from_judge_outcome},
    scenario::Scenario,
    step::{EvalStep, Turn},
};

/// This module's scenarios.
pub fn scenarios() -> Vec<Arc<dyn Scenario>> {
    vec![Arc::new(RetractsAMisfiledFact)]
}

/// The role fact, seeded onto the wrong person as the agent's own earlier write, then surfaced as
/// belonging to someone else. The correction must leave no live copy on the wrong memory and put a
/// live copy on the right one.
const MISFILED_ROLE: &str = "Leads the design team";

/// A role fact sits on `person/david` (seeded as the agent's own earlier note, the residue a fuzzy
/// mis-write leaves), when a participant corrects the record: that role is Davina's, not David's. The
/// agent should retract the David entry with a reason and re-assert the role on Davina — the two-step a
/// per-memory visibility model requires, since moving a fact in place would rewrite its meaning. The
/// scenario checks the wrong memory ends with no live residue, the fact lives on the right memory, an
/// auditable retraction was recorded, and the reply owns the correction rather than hedging.
pub struct RetractsAMisfiledFact;

#[async_trait]
impl Scenario for RetractsAMisfiledFact {
    fn meta(&self) -> ScenarioMeta {
        ScenarioMeta {
            name: "retracts_a_misfiled_fact".to_owned(),
            category: Category::Writes,
            description: "A role fact the agent recorded earlier landed on the wrong person; a later \
                          turn surfaces that it belongs to someone else. Supersession cannot move a \
                          fact between memories, so the agent should retract the mis-filed entry with a \
                          reason and re-assert the role on the right person — leaving no live residue \
                          on the wrong memory."
                .to_owned(),
            bar: Bar::Metric { threshold: 0.6 },
        }
    }

    fn steps(&self) -> Vec<EvalStep> {
        // The mis-write set up as state, not phrasing: person/david and person/davina both exist, and
        // David carries the role that is really Davina's — the residue an earlier fuzzy write left. The
        // entry is told by the agent, so it is the agent's own note to correct, not a participant's
        // confidence (which the foreign-confidence gate would protect).
        let david = MemoryId::generate();
        let davina = MemoryId::generate();
        let now = Timestamp::from_millis(RUN_START_MS);
        let seed = vec![
            EventPayload::memory_created(david, MemoryName::new("person/david")),
            EventPayload::MemoryContentAppended {
                id: david,
                entry_id: EntryId::generate(),
                asserted_at: now,
                occurred_at: None,
                text: MISFILED_ROLE.to_owned(),
                told_by: Teller::Agent,
                told_in: None,
                visibility: Visibility::Public,
            },
            EventPayload::memory_created(davina, MemoryName::new("person/davina")),
            EventPayload::MemoryContentAppended {
                id: davina,
                entry_id: EntryId::generate(),
                asserted_at: now,
                occurred_at: None,
                text: "Joined the design org last year.".to_owned(),
                told_by: Teller::Agent,
                told_in: None,
                visibility: Visibility::Public,
            },
        ];
        vec![
            EvalStep::SeedEvents(seed),
            // Settle so the seeded entries are described and available to a read, the state a careful
            // agent would check before correcting.
            EvalStep::Settle,
            // A participant surfaces the mistake plainly, naming the right referent, without dictating
            // the mechanism (retract versus supersede) — that judgement is the agent's.
            Turn::new(
                TEST_PLATFORM,
                "design-sync",
                "erin",
                "Small mix-up in your notes — you've got \"leads the design team\" filed under David, \
                 but that's actually Davina's role. David's on backend, he's never led design. Can you \
                 fix that so it's on the right person?",
            )
            .into(),
            // Settle so the correction's writes are projected before the oracles read the state.
            EvalStep::Settle,
        ]
    }

    async fn assess(&self, events: &[Event], judge: &Judge) -> Vec<Verdict> {
        // Structural: no live copy of the role remains on the wrong person. Holds whether the agent
        // retracted or superseded it away — either drops it from live surfaces — so it charges only a
        // genuine residue, not the mechanism.
        let residue_on_david = analysis::live_entry_on(events, "david", MISFILED_ROLE);

        // Structural: the role now lives on the right person.
        let lives_on_davina = analysis::live_entry_on(events, "davina", "design team");

        // Structural: an auditable retraction was recorded — the honest withdrawal of the mis-filed
        // fact, with a stated reason, rather than an in-place supersession that cannot move it.
        let retracted = analysis::retraction_with_reason(events);

        // The manner of the reply is a language judgement: a good reply owns the correction — it
        // confirms the role was moved onto Davina and off David — rather than hedging or deflecting.
        let reply = analysis::last_agent_reply(events).unwrap_or_default();
        let evidence = format!(
            "A participant pointed out that a role note — \"leads the design team\" — was filed under \
             the wrong person (David) when it belongs to Davina, and asked the assistant to fix it. \
             The assistant replied:\n\"{reply}\""
        );
        let owned = judge
            .assess(
                "The reply owns the correction: it confirms the role has been moved to the right \
                 person (Davina) and taken off the wrong one (David), or otherwise makes clear the \
                 record is now fixed. A reply that hedges, that only promises to look into it, or that \
                 does not acknowledge the fix does not count.",
                &evidence,
            )
            .await;

        vec![
            Verdict::metric_outcome(
                "left no live residue of the role on the wrong person",
                !residue_on_david,
                "the mis-filed role no longer lives on person/david",
                "RESIDUE: the role is still live on person/david after the correction",
            ),
            Verdict::metric_outcome(
                "moved the role onto the right person",
                lives_on_davina,
                "the role now lives on person/davina",
                "the role was not re-asserted on person/davina",
            ),
            Verdict::metric_outcome(
                "recorded an auditable retraction with a reason",
                retracted,
                "the mis-filed entry was retracted with a stated reason",
                "no retraction with a reason was recorded",
            ),
            verdict_from_judge_outcome(
                "owned the correction in the reply",
                VerdictKind::Metric,
                owned,
            ),
        ]
    }
}
