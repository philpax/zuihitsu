//! Flagging a direct contradiction (migrated from `eval_arbitration.rs`). The deterministic tests prove
//! the mechanism — given an `arbitration` in the synthesize call, a `BeliefArbitrated` is recorded.
//! This asks the harder question only the real model answers: shown two directly contradicting
//! statements, does it flag the conflict rather than silently smoothing it into one description? A
//! tracked quality rate, not a safety gate — conflict detection is a model judgment.

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
                "Given two directly contradicting accounts of the same person in one message, \
                          the agent flags the conflict (a belief arbitration) rather than silently \
                          smoothing it into a single description."
                    .to_owned(),
            bar: Bar::Metric { threshold: 0.5 },
        }
    }

    async fn run(&self, ctx: &RunContext) -> Result<(), EvalError> {
        ctx.turn(Turn::new(
            "discord",
            "leads",
            "dave",
            "Two colleagues gave me contradictory accounts of Dave: Alice says Dave is a committed \
             vegetarian who never eats meat, while Bob insists Dave eats steak every week and isn't \
             vegetarian at all.",
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
