//! Ambient recall: a concept registered under a common word is recalled without the agent being asked
//! to search for it. A participant teaches the agent about their project — named after an everyday word
//! — across a substantive exchange. Days later, in a fresh room, a different participant asks a casual
//! question naming that word. Nothing in the brief carries the project (it fell out of the working-set
//! window), and the question does not say "search your memory", so only the pre-turn ambient recall
//! pass can put the project in front of the agent. The reward is a reply that shows the project
//! entered the agent's awareness — hedging between the two senses of the word is fine, since the
//! asker is new; only a reply with no sign of the project misses.

use std::sync::Arc;

use async_trait::async_trait;
use zuihitsu::{Event, EventPayload, MemoryId, TEST_PLATFORM, TurnId};

use crate::{
    analysis,
    context::MILLIS_PER_DAY,
    judge::Judge,
    package::{Bar, Category, ScenarioMeta, Verdict, VerdictKind, verdict_from_judge_outcome},
    scenario::Scenario,
    step::{EvalStep, Turn},
};

/// This module's scenarios.
pub fn scenarios() -> Vec<Arc<dyn Scenario>> {
    vec![Arc::new(RecallsAConceptByAmbientHint)]
}

pub struct RecallsAConceptByAmbientHint;

#[async_trait]
impl Scenario for RecallsAConceptByAmbientHint {
    fn meta(&self) -> ScenarioMeta {
        ScenarioMeta {
            name: "recalls_a_concept_by_ambient_hint".to_owned(),
            category: Category::Recall,
            description: "A participant teaches the agent about a project named after a common word \
                          (bonsai, a schema-migration tool). Days later, in a fresh room with an empty \
                          buffer, a different participant asks casually what the agent thinks of it. The \
                          project has fallen out of the working set, and nothing asks for a search, so \
                          only the ambient recall pass can surface it — the reply should show \
                          awareness of the project sense, hedging allowed."
                .to_owned(),
            bar: Bar::Metric { threshold: 0.6 },
        }
    }

    fn needs_retrieval(&self) -> bool {
        true
    }

    fn steps(&self) -> Vec<EvalStep> {
        vec![
            // A substantive exchange: Erin teaches the agent about bonsai, her team's schema-migration
            // tool, with a couple of concrete details. Taught in the open as a shared team tool, so it
            // is recorded as public and the lexical index (public-only) can later surface it.
            Turn::new(
                TEST_PLATFORM,
                "eng",
                "erin",
                "Something for you to know about our stack: we run all our database schema changes \
                 through a tool we built in-house called bonsai. It versions each migration, applies \
                 them in order, and can roll one back if a deploy goes sideways.",
            )
            .with_present(&["erin"])
            .into(),
            Turn::new(
                TEST_PLATFORM,
                "eng",
                "erin",
                "The name's a bit of a joke — you prune and shape the schema over time, like a bonsai \
                 tree. Anyway, if anyone mentions bonsai around here they almost always mean the \
                 migration tool, not gardening.",
            )
            .with_present(&["erin"])
            .into(),
            EvalStep::Settle,
            // Enough time passes that the project falls outside the cold-open working-set window (a week
            // by default), so a fresh session will not re-surface it as an active thread — the ambient
            // pass is then the only path to it.
            EvalStep::Advance {
                millis: 8 * MILLIS_PER_DAY,
            },
            // A fresh room, a different participant, an empty buffer: a casual question naming the common
            // word, with no instruction to search. Only ambient recall can connect it to the project.
            Turn::new(
                TEST_PLATFORM,
                "watercooler",
                "marcus",
                "Random question — what do you think of bonsai?",
            )
            .with_present(&["marcus"])
            .into(),
        ]
    }

    async fn assess(&self, events: &[Event], judge: &Judge) -> Vec<Verdict> {
        let reply = analysis::last_agent_reply(events).unwrap_or_default();
        // The answering turn — the last agent reply's turn — is where the ambient hint for the question
        // must have fired.
        let answering_turn = analysis::agent_replies_with_inbound(events)
            .last()
            .map(|(turn_id, _, _)| *turn_id);
        let surfaced =
            answering_turn.is_some_and(|turn_id| ambient_named_bonsai_in_turn(events, turn_id));

        let judged = judge
            .assess(
                "The reply shows the agent knows bonsai as the schema-migration/database tool the \
                 team built — it discusses the tool, or asks whether the tool or the tree is meant, \
                 or covers both senses. The question comes from someone new, so hedging between the \
                 two senses passes, as long as the tool is one of them. Only a reply with no sign of \
                 the tool — answering solely about the tree or gardening, or admitting no knowledge — \
                 fails.",
                &format!(
                    "Days earlier, in another room, the agent was taught that 'bonsai' is the team's \
                     in-house database schema-migration tool (it versions migrations, applies them in \
                     order, and can roll one back). Later, in a fresh room, a different person asked \
                     casually: 'what do you think of bonsai?' The agent replied:\n\"{reply}\""
                ),
            )
            .await;

        vec![
            verdict_from_judge_outcome(
                "showed awareness of the project sense of the word",
                VerdictKind::Metric,
                judged,
            ),
            Verdict::metric_outcome(
                "ambient recall surfaced the project in the answering turn",
                surfaced,
                "an AmbientRecallSurfaced event named the bonsai memory in the answering turn",
                "no ambient hint named the bonsai memory when the question was asked",
            ),
        ]
    }
}

/// Whether an `AmbientRecallSurfaced` event keyed to `turn_id` names a memory whose handle mentions
/// bonsai — the structural evidence that the ambient pass put the project in front of the agent for the
/// answering turn.
fn ambient_named_bonsai_in_turn(events: &[Event], turn_id: TurnId) -> bool {
    let names = analysis::memory_names(events);
    let mentions_bonsai = |memory: &MemoryId| {
        names
            .get(memory)
            .is_some_and(|name| name.to_lowercase().contains("bonsai"))
    };
    events.iter().any(|event| {
        matches!(
            &event.payload,
            EventPayload::AmbientRecallSurfaced { turn_id: hit_turn, hits, .. }
                if *hit_turn == turn_id && hits.iter().any(|hit| mentions_bonsai(&hit.memory))
        )
    })
}
