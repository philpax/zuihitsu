use super::*;

/// A one-off deadline, not a recurrence: the agent is asked to remember a task due "this Friday", the
/// clock crosses Friday, and a fresh-session turn should fire the wake-up and surface the reminder. A
/// task shape the recurring-reminder fixture does not cover — a single dated obligation coming due.
pub struct AReminderComesDue;

#[async_trait]
impl Scenario for AReminderComesDue {
    fn meta(&self) -> ScenarioMeta {
        ScenarioMeta {
            name: "a_reminder_comes_due".to_owned(),
            category: Category::Sessions,
            description: "Asked to remember a one-off task due this Friday, the agent should schedule it; \
                          after the clock crosses Friday, a fresh-session turn should fire the wake-up \
                          and surface the reminder in the reply — a single dated obligation, not a \
                          recurrence."
                .to_owned(),
            bar: Bar::Metric { threshold: 0.5 },
        }
    }

    fn steps(&self) -> Vec<EvalStep> {
        vec![
            Turn::new(
                "discord",
                "team-room",
                "marcus",
                "Don't let me forget — I need to send the board update this Friday. Nudge me about it?",
            )
            .into(),
            // Temporal extraction (which schedules the wake-up) runs off the hot path; drive it before
            // advancing so the reminder is actually scheduled to fire.
            EvalStep::DescribeCatchUp,
            // Cross Friday, then a fresh-session turn fires the due wake-up.
            EvalStep::Advance {
                millis: FIVE_DAYS_MS,
            },
            Turn::new(
                "discord",
                "team-room",
                "marcus",
                "Morning! Anything I should be on top of today?",
            )
            .into(),
        ]
    }

    async fn assess(&self, events: &[Event], judge: &Judge) -> Vec<Verdict> {
        let surfaced = analysis::scheduled_item_surfaced(events);
        let reply = analysis::last_agent_reply(events).unwrap_or_default();
        let delivered = judge
            .assess(
                "The reply reminds the user about sending the board update — the task it was earlier \
                 asked to nudge them about.",
                &format!(
                    "Earlier, the agent was asked to remind the user to send the board update this \
                     Friday. After Friday, asked \"anything I should be on top of today?\", the agent \
                     replied:\n\"{reply}\""
                ),
            )
            .await;

        vec![
            Verdict::oracle_outcome(
                "the one-off wake-up fired and surfaced into a session",
                surfaced,
                "a fired occurrence was raised into a session",
                "no wake-up surfaced after the clock crossed Friday",
            ),
            verdict_from_judge_outcome(
                "surfaced the due reminder to the user in its reply",
                VerdictKind::Metric,
                delivered,
            ),
        ]
    }
}
