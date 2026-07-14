//! Recall across rooms: a public fact recorded in one room must be retrievable, by meaning, in another
//! (migrated from `real_model_recalls_a_fact_by_searching_its_memory`). A quality metric — the model
//! sometimes misses — judged by whether the reply reflects the stored fact.

use std::sync::Arc;

use async_trait::async_trait;
use zuihitsu::Event;

use crate::{
    analysis,
    judge::Judge,
    package::{Bar, Category, ScenarioMeta, Verdict, VerdictKind, verdict_from_judge_outcome},
    scenario::Scenario,
    step::{EvalStep, Turn},
};

/// This module's scenarios.
pub fn scenarios() -> Vec<Arc<dyn Scenario>> {
    vec![Arc::new(Recall), Arc::new(AdmitsAbsence)]
}

pub struct Recall;

#[async_trait]
impl Scenario for Recall {
    fn meta(&self) -> ScenarioMeta {
        ScenarioMeta {
            name: "recall_across_rooms".to_owned(),
            category: Category::Recall,
            description: "A public fact recorded in one room is retrieved, by meaning, when asked \
                          about it in a different room with an empty buffer."
                .to_owned(),
            bar: Bar::Metric { threshold: 0.7 },
        }
    }

    fn needs_retrieval(&self) -> bool {
        true
    }

    fn steps(&self) -> Vec<EvalStep> {
        vec![
            // Turn 1: a public, non-person fact recorded in the team room.
            Turn::new(
                "discord",
                "team-room",
                "dave",
                "Team note to keep for everyone: the Friday standup just moved to 10am, and it's now \
                 held in the Pied Piper conference room.",
            )
            .into(),
            // Regenerate the memory's description off the hot path, then embed both it and the entry,
            // as the background describer and indexer would.
            EvalStep::Settle,
            // Turn 2: a different room, a different participant, an empty buffer — recall is the only
            // path.
            Turn::new(
                "discord",
                "hallway",
                "erin",
                "Hey — do you happen to know when and where the Friday standup is these days?",
            )
            .into(),
        ]
    }

    async fn assess(&self, events: &[Event], judge: &Judge) -> Vec<Verdict> {
        let reply = analysis::last_agent_reply(events).unwrap_or_default();

        let evidence = format!(
            "Earlier, in another room, the agent was told: the Friday standup is at 10am, in the \
             Pied Piper conference room. Later, in a different room, someone asked when and where \
             the Friday standup is. The agent replied:\n\"{reply}\""
        );
        let judged = judge
            .assess(
                "The reply correctly recalls the standup's time (10am) and/or place (the Pied Piper \
                 conference room).",
                &evidence,
            )
            .await;

        // The recall itself is the whole property; the route is deliberately unjudged. There are
        // several legitimate avenues to the fact — a semantic search, a remembered handle read, or
        // the brief itself when its recent facts already carry the answer — and pinning any
        // mechanism punishes a correct recall for its route.
        vec![verdict_from_judge_outcome(
            "recalls the standup details",
            VerdictKind::Metric,
            judged,
        )]
    }
}

/// A person is recorded — a real memory with a real handle — but a specific fact about them is never
/// mentioned. A later session asks that specific question. The whole record is one look away
/// (memory.get the person, or read their details), and it plainly does not hold the answer, so the
/// honest reply admits the absence rather than inventing a fact or grinding search after search hoping
/// something surfaces. The bounded-deliberation metric guards the flailing failure mode: a genuinely
/// absent answer is not a cue to keep searching.
pub struct AdmitsAbsence;

#[async_trait]
impl Scenario for AdmitsAbsence {
    fn meta(&self) -> ScenarioMeta {
        ScenarioMeta {
            name: "admits_absence".to_owned(),
            category: Category::Recall,
            description: "A person is recorded, but a specific fact about them (their dietary \
                          restrictions) never is. Asked that question in a later session, the agent \
                          should read the record, see it holds nothing on it, and say so plainly — \
                          after a bounded look, not endless re-searching."
                .to_owned(),
            bar: Bar::Metric { threshold: 0.7 },
        }
    }

    fn needs_retrieval(&self) -> bool {
        true
    }

    fn steps(&self) -> Vec<EvalStep> {
        vec![
            // Session 1: Priya recorded — a real handle, but nothing about her diet.
            Turn::new(
                "discord",
                "general",
                "marcus",
                "New teammate to note: Priya — she's our incoming design lead, starts next week.",
            )
            .into(),
            EvalStep::Settle,
            // Unrelated chatter — noise before the question, a different room.
            Turn::new(
                "discord",
                "random",
                "erin",
                "The office plants are thriving lately, whoever's been watering them — bless you.",
            )
            .into(),
            EvalStep::Settle,
            // Session 2: an empty buffer, a question whose specific answer was never recorded.
            Turn::new(
                "discord",
                "planning",
                "marcus",
                "Sorting the offsite dinner — do we have anything on file about Priya's dietary \
                 restrictions?",
            )
            .into(),
        ]
    }

    async fn assess(&self, events: &[Event], judge: &Judge) -> Vec<Verdict> {
        let reply = analysis::last_agent_reply(events).unwrap_or_default();
        // Bounded deliberation: how many times the agent embedded a search while answering the absence
        // question — the final exchange's turn, not the whole run, so an earlier turn's legitimate
        // recall does not erode the budget. A handful is fine (read the person, maybe a search or
        // two); a pile is the flailing this guards.
        let searches: usize = analysis::agent_replies_with_inbound(events)
            .last()
            .map(|(turn_id, _, _)| {
                analysis::lua_scripts_for_turn(events, *turn_id)
                    .iter()
                    .map(|script| script.matches("memory.search").count())
                    .sum()
            })
            .unwrap_or(0);
        let bounded = searches <= MAX_ABSENCE_SEARCHES;
        let judged = judge
            .assess(
                "The reply says there is nothing on file about Priya's dietary restrictions — it \
                 admits the absence honestly and does not invent, guess, or imply a restriction that \
                 was never recorded.",
                &format!(
                    "Priya is a recorded teammate, but nothing about her dietary restrictions was ever \
                     mentioned. Asked whether there is anything on file about them, the agent \
                     replied:\n\"{reply}\""
                ),
            )
            .await;

        vec![
            verdict_from_judge_outcome(
                "admitted it holds nothing on the question",
                VerdictKind::Metric,
                judged,
            ),
            Verdict::metric_outcome(
                "kept the look-up bounded",
                bounded,
                format!("{searches} search call(s) — a bounded look"),
                format!("{searches} search calls — re-searching an answer that is not there"),
            ),
        ]
    }
}

/// The most `memory.search` calls a bounded absence look-up should take, counted within the answering
/// turn alone: reading the person and a search or two is fine; past this it is the
/// re-searching-an-absent-answer failure mode.
const MAX_ABSENCE_SEARCHES: usize = 3;
