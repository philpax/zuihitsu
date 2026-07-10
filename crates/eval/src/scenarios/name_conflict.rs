//! Recovering from a name collision. When a `create` reaches for a handle that is already taken, the
//! teachable error names the collision and lists the near-matching existing handles, so the agent
//! picks a distinguishing name (`person/dave-chen` versus `person/dave-patel`) rather than colliding
//! repeatedly or minting a near-duplicate. These scenarios build a cluster of same-stem people across
//! separate sessions — the setup that provokes a collision when the agent reaches for the obvious
//! handle — and reward keeping them as distinct memories instead of overwriting one onto another.

use std::sync::Arc;

use async_trait::async_trait;
use zuihitsu::{Event, Namespace};

use crate::{
    analysis,
    judge::Judge,
    package::{Bar, Category, ScenarioMeta, Verdict, VerdictKind},
    scenario::Scenario,
    step::{EvalStep, Turn},
};

/// This module's scenarios.
pub fn scenarios() -> Vec<Arc<dyn Scenario>> {
    vec![Arc::new(DistinguishesCollidingPeople)]
}

/// Three distinct people who share a first name — three Daves — are introduced in separate sessions,
/// each an empty buffer so the earlier handles are not in front of the agent. Recording the second and
/// third pulls the agent toward the obvious `person/dave` handle, which collides; the right recovery is
/// to pick a distinguishing handle for each, leaving three separate memories, not to fold a new Dave
/// onto an existing one (a wrong merge) or to give up after colliding.
pub struct DistinguishesCollidingPeople;

#[async_trait]
impl Scenario for DistinguishesCollidingPeople {
    fn meta(&self) -> ScenarioMeta {
        ScenarioMeta {
            name: "distinguishes_colliding_people".to_owned(),
            category: Category::Recall,
            description: "Three distinct people who share a first name are introduced across separate \
                          sessions. Recording each pulls the agent toward the same obvious handle, which \
                          collides; it should pick a distinguishing handle for each — keeping three \
                          separate memories — rather than overwrite one Dave onto another or stall on \
                          the collision."
                .to_owned(),
            bar: Bar::Metric { threshold: 0.7 },
        }
    }

    fn needs_retrieval(&self) -> bool {
        true
    }

    fn steps(&self) -> Vec<EvalStep> {
        vec![
            // Session 1: the first Dave — the backend lead.
            Turn::new(
                "discord",
                "general",
                "marcus",
                "Someone to keep track of: Dave — he's our backend lead, been here for years.",
            )
            .into(),
            EvalStep::Settle,
            // Session 2, a different room and an empty buffer: a second, unrelated Dave. Recording him
            // reaches for the same obvious handle as the first, so the create collides.
            Turn::new(
                "discord",
                "design-crit",
                "erin",
                "Adding someone — a different Dave, Dave on the design team who just started this week. \
                 Not the backend Dave, a new hire.",
            )
            .into(),
            EvalStep::Settle,
            // Session 3, another empty buffer: a third Dave again distinct from the first two.
            Turn::new(
                "discord",
                "sales-sync",
                "marcus",
                "And yet another one to remember: Dave in sales — closed the big account this quarter. \
                 Different person from the backend lead and the designer, just happens to share the name.",
            )
            .into(),
            EvalStep::Settle,
            // A later room with an empty buffer asks the agent to tell the three apart — answering well
            // rests on their being three distinct memories, not one Dave overwritten by the next.
            Turn::new(
                "discord",
                "planning",
                "erin",
                "We've got three Daves now and I keep mixing them up — who's who again?",
            )
            .into(),
        ]
    }

    async fn assess(&self, events: &[Event], judge: &Judge) -> Vec<Verdict> {
        // The person memories under the shared stem: exactly the three real people if each collision
        // resolved to a distinguishing handle, fewer if a new Dave was folded onto an existing one (the
        // wrong merge the collision error steers away from), more if a phantom variant was minted.
        let dave_memories: Vec<String> =
            analysis::memories_in_namespace(events, Namespace::Person.prefix())
                .into_iter()
                .filter(|name| name.to_lowercase().contains("dave"))
                .collect();
        let three_distinct = dave_memories.len() == 3;
        let reply = analysis::last_agent_reply(events).unwrap_or_default();
        let judged = judge
            .assess(
                "The reply distinguishes all three Daves as separate people — the backend lead, the \
                 designer (a recent hire), and the one in sales — and does not conflate two of them or \
                 drop one.",
                &format!(
                    "Three different people who share the first name Dave were introduced across \
                     earlier sessions: a long-standing backend lead, a designer who just started, and \
                     one in sales who closed a big account. Asked who the three Daves are, the agent \
                     replied:\n\"{reply}\""
                ),
            )
            .await;

        vec![
            Verdict::metric_outcome(
                "kept the three colliding Daves as distinct memories",
                three_distinct,
                format!("three separate person memories under the shared stem: {dave_memories:?}"),
                format!("the three Daves are not three distinct memories: {dave_memories:?}"),
            ),
            Verdict::from_judge_outcome(
                "told the three Daves apart in its reply",
                VerdictKind::Metric,
                judged,
            ),
        ]
    }
}
