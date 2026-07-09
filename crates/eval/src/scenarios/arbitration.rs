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
    judge::Judge,
    package::{Bar, Category, ScenarioMeta, Verdict},
    scenario::Scenario,
    step::{EvalStep, Turn},
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

    fn steps(&self) -> Vec<EvalStep> {
        vec![
            // Turn 1: establish one public account of where the Q3 offsite is.
            Turn::new(
                "discord",
                "leads",
                "marcus",
                "For the Q3 planning notes: the team offsite this year is happening in Denver.",
            )
            .into(),
            EvalStep::DescribeCatchUp,
            // Turn 2: a different teller gives an independent, equally-confident account of the same fact
            // that happens to conflict — phrased as her own standing understanding, not a correction of
            // Marcus's, and carrying no recency or authority cue ("locked in last week", "just confirmed")
            // that would license treating it as a newer, truer fact. With both accounts co-equal, the agent
            // has no ground to supersede one with the other; the conflict must land as a second standing
            // public entry the synthesis reconciles. (A recency cue here invites a defensible supersession
            // instead, which would test the wrong thing.)
            Turn::new(
                "discord",
                "leads",
                "erin",
                "For the Q3 offsite, I've got it down as Austin on my end — please put Austin in the \
                 planning notes.",
            )
            .into(),
            // Belief arbitration is synthesized off the hot path; drive it before the log is assessed.
            EvalStep::DescribeCatchUp,
        ]
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
