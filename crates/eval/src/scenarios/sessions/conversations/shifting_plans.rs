use super::*;

/// A planning thread where a date changes under the agent: a launch is penciled in, then slips a week
/// with the first date explicitly scrapped, then — from another room — someone asks the current date.
/// An explicit correction is an *update*, not a standing contradiction (so the synthesis should not
/// arbitrate it); the right move is to supersede the stale date and answer with the current one.
/// Intermingles update handling, temporal reasoning, and cross-room recall.
pub struct ShiftingPlans;

#[async_trait]
impl Scenario for ShiftingPlans {
    fn meta(&self) -> ScenarioMeta {
        ScenarioMeta {
            name: "shifting_plans".to_owned(),
            category: Category::Sessions,
            description: "A launch date is set, then slips a week with the first date explicitly \
                          scrapped, then asked from another room. The agent should supersede the stale \
                          date (an explicit correction is an update, not a contradiction to arbitrate) \
                          and answer with the corrected date — update handling, temporal reasoning, and \
                          recall in one thread."
                .to_owned(),
            bar: Bar::Metric { threshold: 0.6 },
        }
    }

    fn needs_retrieval(&self) -> bool {
        true
    }

    fn steps(&self) -> Vec<EvalStep> {
        vec![
            // The launch is penciled in.
            Turn::new(
                "discord",
                "planning",
                "marcus",
                "Let's pencil in the product launch for the 15th of March.",
            )
            .into(),
            EvalStep::DescribeCatchUp,
            // A day later, it slips — and the first date is explicitly scrapped (a direct contradiction).
            EvalStep::Advance {
                millis: MILLIS_PER_DAY,
            },
            Turn::new(
                "discord",
                "planning",
                "marcus",
                "Actually, scratch that — the launch has slipped to the 22nd of March. The 15th is off.",
            )
            .into(),
            // The contradiction is reconciled off the hot path (the arbitration pass), then embedded.
            EvalStep::Settle,
            // From another room, with an empty buffer, someone asks the current date — recall plus the
            // reconciled belief.
            Turn::new(
                "discord",
                "standup",
                "erin",
                "Quick one — what's the current launch date now?",
            )
            .into(),
        ]
    }

    async fn assess(&self, events: &[Event], judge: &Judge) -> Vec<Verdict> {
        let superseded = analysis::any_superseded(events);

        let reply = analysis::last_agent_reply(events).unwrap_or_default();
        let evidence = format!(
            "A launch was first set for the 15th of March, then explicitly moved to the 22nd of March \
             (the 15th was scrapped). Later, in a different room, someone asked what the current launch \
             date is. The agent replied:\n\"{reply}\""
        );
        let judged = judge
            .assess(
                "The reply gives the launch date as the 22nd of March (the corrected date), and does \
                 not present the 15th as the current launch date.",
                &evidence,
            )
            .await;

        vec![
            verdict_from_judge_outcome(
                "answered with the corrected launch date, not the stale one",
                VerdictKind::Metric,
                judged,
            ),
            Verdict::oracle_outcome(
                "superseded the stale date rather than leaving it standing",
                superseded,
                "the original date entry was superseded by the correction",
                "the stale date was left standing (or only the reply was updated)",
            ),
        ]
    }
}
