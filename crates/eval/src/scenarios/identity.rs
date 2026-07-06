//! Operator identity across the imprint boundary. The imprint interview is one-time onboarding: it
//! establishes the operator's profile and merges the provisional `person/operator` stub into it. Once
//! that first session has closed, a later session in which the operator shares another of their names
//! should record it on the existing profile, not mint a second `person/*`. The bug this guards against
//! is the operator channel re-running the imprint's create-a-profile script on every turn, so each new
//! name the operator gives spawns a fresh profile that is then `same_as`-merged.

use std::sync::Arc;

use async_trait::async_trait;
use zuihitsu::{Event, MemoryName, NamespacedMemoryName};

use crate::{
    analysis,
    context::{PAST_IDLE_GAP_MS, RunContext},
    error::EvalError,
    judge::Judge,
    package::{Bar, Category, ScenarioMeta, Verdict},
    scenario::Scenario,
};

/// This module's scenarios.
pub fn scenarios() -> Vec<Arc<dyn Scenario>> {
    vec![Arc::new(OperatorSecondNameLandsOnTheExistingProfile)]
}

/// The operator imprints, introducing themselves by a handle — the agent records a profile and merges
/// the provisional `person/operator` stub into it. The session then lapses and a fresh one opens, in
/// which the operator shares their real name. The agent should append it to the existing profile, not
/// mint a second `person/*` — the imprint's create-a-profile is a one-time onboarding step, not a
/// standing instruction the operator channel re-runs each turn.
pub struct OperatorSecondNameLandsOnTheExistingProfile;

#[async_trait]
impl Scenario for OperatorSecondNameLandsOnTheExistingProfile {
    fn meta(&self) -> ScenarioMeta {
        ScenarioMeta {
            name: "operator_second_name_lands_on_the_existing_profile".to_owned(),
            category: Category::Relations,
            description: "After the imprint establishes the operator's profile, a later session in \
                          which they share another of their names should record it on that profile, \
                          not mint a second person/* memory."
                .to_owned(),
            bar: Bar::gating(),
        }
    }

    async fn run(&self, ctx: &RunContext) -> Result<(), EvalError> {
        // Imprint: the operator introduces themselves by a handle. The agent records a profile and
        // merges the provisional person/operator stub into it.
        ctx.imprint(
            "Hi! I'm rowan. You'll help me run my bakery — keeping track of suppliers, regulars, \
             and the day-to-day.",
        )
        .await?;
        ctx.imprint("We're a small place, busiest on weekends, known for our sourdough.")
            .await?;
        ctx.describe_catch_up().await?;

        // The session lapses; a fresh one opens. The operator shares their real name — it should be
        // added to the existing profile, not start a new one.
        ctx.advance(PAST_IDLE_GAP_MS);
        ctx.imprint("Oh, by the way — my real name is Tomas, in case that's useful to record.")
            .await?;
        ctx.describe_catch_up().await?;
        Ok(())
    }

    async fn assess(&self, events: &[Event], _judge: &Judge) -> Vec<Verdict> {
        // The desired move: append the real name as a fact on the existing operator profile — neither
        // a second stub nor a rename of the handle. Three gates: no duplicate, no rename, name recorded.
        let profiles = analysis::memories_in_namespace(events, "person/");
        let operator = MemoryName::from(NamespacedMemoryName::operator());
        let recorded_name = analysis::entries(events).iter().any(|entry| {
            entry.memory.starts_with("person/")
                // The real profile, not the provisional person/operator anchor.
                && entry.memory != operator.as_str()
                && entry.text.to_lowercase().contains("tomas")
        });
        vec![
            Verdict::oracle_outcome(
                "kept a single operator profile, minting no duplicate",
                // person/operator plus the one profile the imprint created is the whole identity.
                profiles.len() <= 2,
                "no duplicate operator profile was created",
                format!("minted a duplicate operator profile — person/* memories: {profiles:?}"),
            ),
            Verdict::oracle_outcome(
                "recorded the real name without renaming the handle away",
                !analysis::memory_renamed(events),
                "kept the established handle and did not rename it",
                "renamed the operator's handle instead of recording the real name as a fact",
            ),
            Verdict::oracle_outcome(
                "recorded the real name as a fact on the existing profile",
                recorded_name,
                "the real name was appended to the operator's profile",
                "the real name was not recorded as a fact on the operator's profile",
            ),
        ]
    }
}
