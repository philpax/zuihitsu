//! Flagging a contradiction between accumulated beliefs (migrated from `eval_arbitration.rs`). Belief
//! arbitration fires when the turn-end synthesis finds a memory's `Public` entries conflicting and
//! reconciles them — so the conflict has to arrive as two separate public entries, not one summary.
//! Two tellers give conflicting accounts of the same non-person fact across two turns (public by
//! default), and the question is whether the model flags the conflict rather than silently smoothing
//! it. A tracked quality rate, not a safety gate — conflict detection is a model judgment.

use std::sync::Arc;

use async_trait::async_trait;
use zuihitsu::Event;

use crate::{
    analysis,
    context::{RunContext, Turn},
    error::EvalError,
    judge::Judge,
    package::{Bar, Category, ScenarioMeta, Verdict},
    scenario::Scenario,
};

/// This module's scenarios.
pub fn scenarios() -> Vec<Arc<dyn Scenario>> {
    vec![Arc::new(Contradiction)]
}

pub struct Contradiction;

#[async_trait]
impl Scenario for Contradiction {
    fn meta(&self) -> ScenarioMeta {
        ScenarioMeta {
            name: "flags_a_contradiction".to_owned(),
            category: Category::Arbitration,
            description:
                "Told a non-person fact and then a conflicting account of it from a different teller \
                          across two turns (so both land as public entries on one memory), the agent \
                          flags the conflict as a belief arbitration rather than silently smoothing it \
                          into a single description."
                    .to_owned(),
            bar: Bar::Metric { threshold: 0.5 },
        }
    }

    async fn run(&self, ctx: &RunContext) -> Result<(), EvalError> {
        // Turn 1: establish one public account of where the Q3 offsite is.
        ctx.turn(Turn::new(
            "discord",
            "leads",
            "phil",
            "For the Q3 planning notes: the team offsite this year is happening in Denver.",
        ))
        .await?;
        ctx.describe_catch_up().await?;
        // Turn 2: a different teller gives an independent, confident account of the same fact that
        // happens to conflict — phrased as its own claim, not a correction of Phil's, so it lands as a
        // second standing public entry the synthesis must reconcile rather than a supersession.
        ctx.turn(Turn::new(
            "discord",
            "leads",
            "erin",
            "For the Q3 offsite, I've got Austin on my end — marketing locked in the Austin venue last \
             week, so put that in the planning notes.",
        ))
        .await?;
        // Belief arbitration is synthesized off the hot path; drive it before the log is assessed.
        ctx.describe_catch_up().await?;
        Ok(())
    }

    async fn assess(&self, events: &[Event], _judge: &Judge) -> Vec<Verdict> {
        let arbitrations = analysis::arbitrations(events);
        let flagged = !arbitrations.is_empty();
        vec![Verdict::metric_outcome(
            "flagged the contradiction as a belief arbitration",
            flagged,
            format!("arbitration statements: {arbitrations:?}"),
            "smoothed the contradiction into one description, no arbitration recorded",
        )]
    }
}
