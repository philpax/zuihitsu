//! Name changes: the agent follows a person who changes the name they go by — a transition above all —
//! by renaming their existing memory, so the person stays one continuous identity rather than splitting
//! into a second, unreconcilable node (spec §Identity → Renaming). `ARenameHoldsUp` threads a rename
//! through several conversations and checks the person's relationships and history carry across it;
//! `ARenamedPersonIsRecognizedByTheirOldName` checks the agent still bridges the prior name — spoken by
//! someone who hasn't heard of the change — to the renamed person, under the new name.

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

/// A couple of days between phases, so each lands in its own session and the clock plainly moves on.
const PHASE_GAP_MS: i64 = 2 * 24 * 60 * 60 * 1000;

/// This module's scenarios.
pub fn scenarios() -> Vec<Arc<dyn Scenario>> {
    vec![
        Arc::new(ARenameHoldsUp),
        Arc::new(ARenamedPersonIsRecognizedByTheirOldName),
    ]
}

/// The full arc: Dave introduces himself, Erin says she knows him; in a later conversation Dave
/// transitions and goes by Sarah; in a later one still, a newcomer asks who Sarah is. A correct agent
/// renamed Dave's memory, so Sarah is the same person Erin knows — the friendship survived the rename —
/// and the agent answers under the new name rather than treating Sarah as a stranger.
pub struct ARenameHoldsUp;

#[async_trait]
impl Scenario for ARenameHoldsUp {
    fn meta(&self) -> ScenarioMeta {
        ScenarioMeta {
            name: "a_rename_holds_up_across_conversations".to_owned(),
            category: Category::Recall,
            description: "Dave introduces himself and Erin says she knows him; later he transitions \
                          and goes by Sarah; later still a newcomer asks who Sarah is. The agent \
                          should have renamed Dave's memory, so Sarah is the same person Erin knows."
                .to_owned(),
            bar: Bar::Metric { threshold: 0.5 },
        }
    }

    fn needs_retrieval(&self) -> bool {
        true
    }

    async fn run(&self, ctx: &RunContext) -> Result<(), EvalError> {
        // Each phase is its own conversation in a different room, with time passing between, so the
        // agent cannot read a shared buffer — connecting them across the rename forces retrieval.
        // Dave introduces himself.
        ctx.turn(Turn::new(
            "discord",
            "onboarding",
            "dave",
            "Hi, I'm Dave — just started on the team this week.",
        ))
        .await?;
        ctx.describe_catch_up().await?;
        ctx.index_catch_up().await?;
        ctx.advance(PHASE_GAP_MS);
        // A separate conversation: Erin says she knows him — the agent must retrieve Dave to attach it.
        ctx.turn(Turn::new(
            "discord",
            "lunch",
            "erin",
            "Speaking of the new folks — Dave and I go way back, we went to college together.",
        ))
        .await?;
        ctx.describe_catch_up().await?;
        ctx.index_catch_up().await?;
        ctx.advance(PHASE_GAP_MS);
        // A separate conversation: Dave transitions and asks to be called Sarah.
        ctx.turn(Turn::new(
            "discord",
            "dave-dm",
            "dave",
            "Hey — I've transitioned, and I go by Sarah now (she/her). Please use that from here on.",
        ))
        .await?;
        ctx.describe_catch_up().await?;
        ctx.index_catch_up().await?;
        ctx.advance(PHASE_GAP_MS);
        // A separate conversation: a newcomer asks who Sarah is and whether anyone knows her.
        ctx.turn(Turn::new(
            "discord",
            "hallway",
            "phil",
            "I keep hearing the name Sarah around here — who is she, and does anyone know her well?",
        ))
        .await?;
        Ok(())
    }

    async fn assess(&self, events: &[Event], judge: &Judge) -> Vec<Verdict> {
        let reply = analysis::last_agent_reply(events).unwrap_or_default();
        let renamed = analysis::memory_renamed(events);

        let evidence = format!(
            "Earlier, Dave introduced himself to the team, and Erin said she knows him well — they \
             went to college together. In a later conversation Dave said he had transitioned and now \
             goes by Sarah. Later still, a newcomer asked who Sarah is and whether anyone knows her. \
             The agent replied:\n\"{reply}\""
        );
        let judged = judge
            .assess(
                "The reply treats Sarah as the same person who joined the team, recalling that Erin \
                 knows her (they go back to college) — so the relationship carried across the name \
                 change — and refers to her as Sarah. It fails if it treats Sarah as a brand-new, \
                 unknown person, says no one knows her, or cannot connect her to Erin.",
                &evidence,
            )
            .await;

        vec![
            Verdict::from_judge_outcome(
                "Sarah is the same person Erin knows",
                VerdictKind::Metric,
                judged,
            ),
            Verdict::metric_outcome(
                "renamed rather than creating a new person",
                renamed,
                "renamed the existing memory instead of minting a second person",
                "did not rename — likely split the person into a new, unrelated memory",
            ),
        ]
    }
}

/// After a rename, someone who hasn't heard of the change uses the old name. The agent should bridge it
/// to the renamed person — the prior name still appears in the history it recorded — and answer under
/// the new name, rather than treating the old name as an unknown stranger.
pub struct ARenamedPersonIsRecognizedByTheirOldName;

#[async_trait]
impl Scenario for ARenamedPersonIsRecognizedByTheirOldName {
    fn meta(&self) -> ScenarioMeta {
        ScenarioMeta {
            name: "a_renamed_person_is_recognized_by_their_old_name".to_owned(),
            category: Category::Recall,
            description:
                "Dave goes by Sarah after a transition; later someone who only knew the old \
                          name asks after Dave. The agent should recognize Dave is now Sarah and \
                          answer under the new name, not as an unknown person."
                    .to_owned(),
            bar: Bar::Metric { threshold: 0.5 },
        }
    }

    fn needs_retrieval(&self) -> bool {
        true
    }

    async fn run(&self, ctx: &RunContext) -> Result<(), EvalError> {
        // Independent conversations across rooms and time, so the final answer must come from memory.
        // Dave introduces himself and what he does.
        ctx.turn(Turn::new(
            "discord",
            "ops",
            "dave",
            "Hey, I'm Dave — I handle the deploys around here.",
        ))
        .await?;
        ctx.describe_catch_up().await?;
        ctx.index_catch_up().await?;
        ctx.advance(PHASE_GAP_MS);
        // A separate conversation: Dave transitions and goes by Sarah.
        ctx.turn(Turn::new(
            "discord",
            "dave-dm",
            "dave",
            "Heads up: I've transitioned and go by Sarah now (she/her) — please use Sarah.",
        ))
        .await?;
        ctx.describe_catch_up().await?;
        ctx.index_catch_up().await?;
        ctx.advance(PHASE_GAP_MS);
        // A separate conversation: someone who only knew the old name asks after Dave.
        ctx.turn(Turn::new(
            "discord",
            "hallway",
            "frank",
            "Is Dave still the one who handles deploys? Haven't seen him around lately.",
        ))
        .await?;
        Ok(())
    }

    async fn assess(&self, events: &[Event], judge: &Judge) -> Vec<Verdict> {
        let reply = analysis::last_agent_reply(events).unwrap_or_default();

        let evidence = format!(
            "Earlier, Dave (who handles deploys) said he had transitioned and now goes by Sarah. \
             Later, in another room, Frank — who only knew the old name — asked whether Dave still \
             handles deploys. The agent replied:\n\"{reply}\""
        );
        let judged = judge
            .assess(
                "The reply recognizes that 'Dave' is the person now called Sarah — it connects the \
                 old name to the same person rather than treating Dave as an unknown stranger — and \
                 refers to her by the current name, Sarah, when answering. It fails if it cannot link \
                 the old name to her, or treats Dave and Sarah as two different people.",
                &evidence,
            )
            .await;

        vec![Verdict::from_judge_outcome(
            "bridges the old name to the renamed person",
            VerdictKind::Metric,
            judged,
        )]
    }
}
