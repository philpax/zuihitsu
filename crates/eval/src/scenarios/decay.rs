//! Decay (spec §Recency and volatility): a fast-changing fact the agent marked `High` volatility ages past
//! usefulness and reads as `[stale]`, so the agent surfaces it as possibly out of date rather than
//! asserting it as current. Exercises both halves: the agent classifying a volatile memory
//! (`set_volatility`), and hedging on the aged fact weeks later.

use std::sync::Arc;

use async_trait::async_trait;
use zuihitsu::Event;

use crate::{
    analysis,
    context::{MILLIS_PER_DAY, RunContext, Turn},
    error::EvalError,
    judge::Judge,
    package::{Bar, Category, ScenarioMeta, Verdict, VerdictKind},
    scenario::Scenario,
};

/// This module's scenarios.
pub fn scenarios() -> Vec<Arc<dyn Scenario>> {
    vec![Arc::new(AVolatileStatusGoesStale)]
}

/// A clearly temporary status (a current rotation, not a permanent base) is recorded, then weeks pass.
/// The agent should mark the memory fast-changing as it records — the primary signal here — so the
/// fact later ages into `[stale]`, and treat it as possibly out of date when asked rather than current.
pub struct AVolatileStatusGoesStale;

#[async_trait]
impl Scenario for AVolatileStatusGoesStale {
    fn meta(&self) -> ScenarioMeta {
        ScenarioMeta {
            name: "a_volatile_status_goes_stale".to_owned(),
            category: Category::Recall,
            description: "A transient status about someone is recorded, then weeks pass. Asked later, \
                          the agent should mark the memory fast-changing so the fact ages into stale, \
                          and surface it as possibly out of date rather than asserting it as current."
                .to_owned(),
            bar: Bar::Metric { threshold: 0.5 },
        }
    }

    fn needs_retrieval(&self) -> bool {
        true
    }

    async fn run(&self, ctx: &RunContext) -> Result<(), EvalError> {
        // A flat, non-time-bound but volatile fact about Dave: a current project lead. Nothing in the
        // wording dates it, so a later hedge must come from the staleness marker, not date-reasoning —
        // the agent has to recognize a current role as fast-changing and mark it as it records.
        ctx.turn(Turn::new(
            "discord",
            "team",
            "marcus",
            "One to keep track of: Dave's the lead on the Atlas project — that's the main thing he's \
             running on the team.",
        ))
        .await?;
        ctx.describe_catch_up().await?;
        ctx.index_catch_up().await?;

        // Two months pass — well past the staleness horizon for a fast-changing fact.
        ctx.advance(60 * MILLIS_PER_DAY);

        // A different person asks what Dave is working on, in a fresh room: recall surfaces the aged fact.
        ctx.turn(
            Turn::new(
                "discord",
                "hallway",
                "erin",
                "What's Dave leading these days? I want to loop him in on something.",
            )
            .with_present(&["erin"]),
        )
        .await?;
        Ok(())
    }

    async fn assess(&self, events: &[Event], judge: &Judge) -> Vec<Verdict> {
        let reply = analysis::last_agent_reply(events).unwrap_or_default();
        vec![
            Verdict::metric_outcome(
                "marked the fast-changing status memory high-volatility",
                analysis::volatility_set_high(events),
                "set a memory to high volatility so its facts can age into stale",
                "left every memory at the default volatility — the status can never read as stale",
            ),
            Verdict::from_judge_outcome(
                "treated the aged role as possibly out of date, not current",
                VerdictKind::Metric,
                judge
                    .assess(
                        "The reply recalls that Dave was leading / running the Atlas project, and \
                         treats that as possibly out of date — hedging (\"last I knew\"), noting it \
                         may have changed, or offering to confirm. Stating as a settled current fact \
                         that Dave leads Atlas fails; so does not recalling the Atlas project at all.",
                        &format!(
                            "Two months ago, someone mentioned Dave was the lead on the Atlas project. \
                             Later, asked what Dave is leading these days, the agent \
                             replied:\n\"{reply}\""
                        ),
                    )
                    .await,
            ),
        ]
    }
}
