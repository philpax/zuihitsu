//! Recall across rooms: a public fact recorded in one room must be retrievable, by meaning, in another
//! (migrated from `real_model_recalls_a_fact_by_searching_its_memory`). A quality metric — the model
//! sometimes misses — judged by whether the reply reflects the stored fact.

use std::sync::Arc;

use async_trait::async_trait;
use zuihitsu::Event;

use crate::{
    analysis,
    context::{RunContext, Turn},
    error::EvalError,
    judge::Judge,
    package::{Bar, Category, ScenarioMeta, Verdict, VerdictKind},
    scenario::Scenario,
};

/// This module's scenarios.
pub fn scenarios() -> Vec<Arc<dyn Scenario>> {
    vec![Arc::new(Recall)]
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

    async fn run(&self, ctx: &RunContext) -> Result<(), EvalError> {
        // Turn 1: a public, non-person fact recorded in the team room.
        ctx.turn(Turn::new(
            "discord",
            "team-room",
            "dave",
            "Team note to keep for everyone: the Friday standup just moved to 10am, and it's now \
             held in the Pied Piper conference room.",
        ))
        .await?;
        // Embed what was written, as the background indexer would.
        ctx.index_catch_up().await?;
        // Turn 2: a different room, a different participant, an empty buffer — recall is the only path.
        ctx.turn(Turn::new(
            "discord",
            "hallway",
            "erin",
            "Hey — do you happen to know when and where the Friday standup is these days?",
        ))
        .await?;
        Ok(())
    }

    async fn assess(&self, events: &[Event], judge: &Judge) -> Vec<Verdict> {
        let reply = analysis::last_agent_reply(events).unwrap_or_default();
        let searched = analysis::lua_called(events, "memory.search");

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

        vec![
            Verdict::from_judge_outcome("recalls the standup details", VerdictKind::Metric, judged),
            Verdict::metric_outcome(
                "reached for memory.search",
                searched,
                "called memory.search",
                "answered without calling memory.search",
            ),
        ]
    }
}
