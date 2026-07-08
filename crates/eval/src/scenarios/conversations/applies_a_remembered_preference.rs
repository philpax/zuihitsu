use super::*;

/// A different task shape: *applying* remembered context to a later request, not just reciting a fact.
/// Someone states a standing dietary preference in passing, the agent banks it, and — turns and a room
/// later — is asked for a lunch recommendation by someone else. A good answer reflects the preference
/// without being reminded of it. Exercises recall plus acting on what was recalled.
pub struct AppliesARememberedPreference;

#[async_trait]
impl Scenario for AppliesARememberedPreference {
    fn meta(&self) -> ScenarioMeta {
        ScenarioMeta {
            name: "applies_a_remembered_preference".to_owned(),
            category: Category::Recall,
            description: "A standing preference (vegetarian) is mentioned in passing, then a lunch \
                          recommendation is asked for from another room without restating it. The reply \
                          should apply the remembered preference, not just recall it — recall put to use."
                .to_owned(),
            bar: Bar::Metric { threshold: 0.6 },
        }
    }

    fn needs_retrieval(&self) -> bool {
        true
    }

    async fn run(&self, ctx: &RunContext) -> Result<(), EvalError> {
        // Marcus mentions a standing preference in passing — not a question, just context to bank.
        ctx.turn(Turn::new(
            "discord",
            "general",
            "marcus",
            "Oh, while I think of it — I'm vegetarian, have been for years. Worth remembering for \
             whenever food comes up.",
        ))
        .await?;
        ctx.settle().await?;

        // Later, a different room, Marcus asks for a lunch spot — without restating the preference. A good
        // answer applies what it banked rather than suggesting a steakhouse.
        ctx.turn(Turn::new(
            "discord",
            "lunch-plans",
            "marcus",
            "I'm starving — got any suggestions for where I should grab lunch today?",
        ))
        .await?;
        Ok(())
    }

    async fn assess(&self, events: &[Event], judge: &Judge) -> Vec<Verdict> {
        // Recalling the preference is implied by applying it: the request is in a different room with an
        // empty buffer, so a reply that reflects the preference can only have come from memory (whether
        // the agent reached it by search or by the person's handle). So judge the outcome, not the call.
        let reply = analysis::last_agent_reply(events).unwrap_or_default();
        let evidence = format!(
            "Earlier, the person mentioned they are vegetarian. Later, in a different room, they asked \
             for a lunch suggestion without restating that. The agent replied:\n\"{reply}\""
        );
        let judged = judge
            .assess(
                "The reply lets the person's vegetarian preference shape the suggestion: it recalls the \
                 preference and tailors to it — offering vegetarian or vegetarian-friendly options, or \
                 naming the preference. It FAILS only if it ignores the preference entirely or steers \
                 them somewhere clearly meat-defined (a steakhouse, a barbecue joint) with no vegetarian \
                 consideration. Judge whether the remembered preference shaped the reply, NOT whether \
                 every option named is exclusively vegetarian — a place that also serves non-vegetarian \
                 food (most restaurants, sushi, a café) does not by itself fail when the reply has \
                 plainly accounted for the preference.",
                &evidence,
            )
            .await;

        vec![Verdict::from_judge_outcome(
            "applied the remembered dietary preference to the recommendation",
            VerdictKind::Metric,
            judged,
        )]
    }
}
