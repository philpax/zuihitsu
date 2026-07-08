use super::*;

/// A genuine disagreement, not a correction: two people give conflicting accounts of where an event is
/// held, and neither retracts. Both accounts should stand — overwriting one would silently resolve a
/// live disagreement to whoever spoke last — so the synthesis should arbitrate, and when asked from
/// another room the agent should surface the discrepancy rather than confidently pick one. The mirror
/// of [`ShiftingPlans`]: an explicit correction is superseded, a standing disagreement is arbitrated,
/// and telling the two apart is the capability under test.
pub struct ConflictingAccounts;

#[async_trait]
impl Scenario for ConflictingAccounts {
    fn meta(&self) -> ScenarioMeta {
        ScenarioMeta {
            name: "conflicting_accounts".to_owned(),
            category: Category::Arbitration,
            description: "Two people give conflicting accounts of where the all-hands is held, neither \
                          retracting. Both should stand (not overwrite to whoever spoke last), the \
                          synthesis should arbitrate the contradiction, and asked from another room the \
                          agent should surface the discrepancy rather than confidently pick one."
                .to_owned(),
            bar: Bar::Metric { threshold: 0.5 },
        }
    }

    fn needs_retrieval(&self) -> bool {
        true
    }

    async fn run(&self, ctx: &RunContext) -> Result<(), EvalError> {
        // Marcus states a location.
        ctx.turn(Turn::new(
            "discord",
            "team-room",
            "marcus",
            "Heads up for everyone: the all-hands next week is in the main auditorium.",
        ))
        .await?;
        // Erin contradicts it — a genuine conflicting account, no retraction by Marcus. She asserts a
        // firm competing *present* belief, not an announced change ("it got moved to ...") and not a
        // hedged "I'm not sure": the two accounts stand side by side as a flat contradiction, so the
        // right operation is to arbitrate (keeping both), not to supersede one with a newer value, and
        // not to soften it into "unconfirmed" prose the synthesis narrates but never records.
        ctx.turn(
            Turn::new(
                "discord",
                "team-room",
                "erin",
                "Wait, that's not what I heard — it's in the rooftop terrace, not the auditorium. \
                 Let's get that straightened out.",
            )
            .with_present(&["marcus", "erin"]),
        )
        .await?;
        // Reconcile off the hot path (the arbitration pass), then embed for cross-room recall.
        ctx.settle().await?;
        // From another room, someone asks where it is — the agent should not silently pick a side.
        ctx.turn(Turn::new(
            "discord",
            "hallway",
            "frank",
            "Quick q — do you know where the all-hands is being held?",
        ))
        .await?;
        Ok(())
    }

    async fn assess(&self, events: &[Event], judge: &Judge) -> Vec<Verdict> {
        // A genuine both-stand arbitration (crediting neither account), not merely any arbitration: an
        // arbitration that credits a side is supersession by another name, which is what the scenario
        // says not to do ("both should stand, not overwrite to whoever spoke last").
        let arbitrated = analysis::both_stand_arbitration(events);
        let reply = analysis::last_agent_reply(events).unwrap_or_default();
        let judged = judge
            .assess(
                "The reply surfaces the disagreement about the all-hands location — it conveys that \
                 there are two accounts (main auditorium vs rooftop terrace) or that it is unsettled / \
                 worth confirming, rather than confidently asserting just one location as settled fact.",
                &format!(
                    "Two people gave conflicting accounts of the all-hands location — one said the main \
                     auditorium, the other the rooftop terrace, neither retracting. Asked from another \
                     room where it is, the agent replied:\n\"{reply}\""
                ),
            )
            .await;

        vec![
            Verdict::metric_outcome(
                "kept both accounts standing and recorded an arbitration",
                arbitrated,
                "the contradiction was held as two entries and arbitrated",
                "no arbitration recorded — the disagreement was overwritten or dropped",
            ),
            Verdict::from_judge_outcome(
                "surfaced the discrepancy rather than confidently picking one",
                VerdictKind::Metric,
                judged,
            ),
        ]
    }
}
