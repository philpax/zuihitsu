use crate::scenarios::sessions::conversations::*;

/// A reminder armed well before it surfaces: the agent is asked to nudge about a task, more than a week
/// passes, and only then — at a fresh session after the user has been away — does the wake-up fire. The
/// surfaced reminder now carries its recording age ("recorded 1 week ago"), so the agent should relay it
/// as the standing reminder it set earlier, not as fresh news it has just come across. This guards the
/// live failure the age stamps address: a days-old wake-up relayed as breaking news, its framing and
/// details both wrong.
pub struct AnOldReminderIsNotFreshNews;

/// Twelve days — long enough that a "this Friday" reminder armed from the Monday anchor has been on the
/// books for over a week when it finally surfaces, so its recording age reads as clearly past, not fresh.
const TWELVE_DAYS_MS: i64 = 12 * MILLIS_PER_DAY;

#[async_trait]
impl Scenario for AnOldReminderIsNotFreshNews {
    fn meta(&self) -> ScenarioMeta {
        ScenarioMeta {
            name: "an_old_reminder_is_not_fresh_news".to_owned(),
            category: Category::Sessions,
            description: "A reminder armed for this Friday surfaces only after more than a week has \
                          passed. The agent should relay it as the reminder it set earlier — grounded \
                          in when it was recorded — rather than presenting a days-old wake-up as fresh \
                          news it has just encountered."
                .to_owned(),
            bar: Bar::Metric { threshold: 0.5 },
        }
    }

    fn steps(&self) -> Vec<EvalStep> {
        vec![
            Turn::new(
                TEST_PLATFORM,
                "team-room",
                "marcus",
                "Before I forget — I need to send the signed vendor contract back this Friday. Nudge me \
                 about it?",
            )
            .into(),
            // Temporal extraction schedules the wake-up off the hot path; drive it before advancing so
            // the reminder is actually armed.
            EvalStep::DescribeCatchUp,
            // Let more than a week pass, so when the wake-up fires its recording age reads as clearly
            // past rather than moments old.
            EvalStep::Advance {
                millis: TWELVE_DAYS_MS,
            },
            Turn::new(
                TEST_PLATFORM,
                "team-room",
                "marcus",
                "Morning — I've been away a while and I'm catching up. Anything outstanding I should \
                 deal with?",
            )
            .into(),
        ]
    }

    async fn assess(&self, events: &[Event], judge: &Judge) -> Vec<Verdict> {
        let surfaced = analysis::scheduled_item_surfaced(events);
        let reply = analysis::last_agent_reply(events).unwrap_or_default();
        let material = format!(
            "Twelve days ago, the agent was asked to remind the user to send the signed vendor contract \
             back this Friday. Now, at a fresh session where the user says they have been away a while \
             and asks \"anything outstanding I should deal with?\", the agent replied:\n\"{reply}\""
        );
        let delivered = judge
            .assess(
                "The reply reminds the user about sending the signed vendor contract back — the task it \
                 was earlier asked to nudge them about.",
                &material,
            )
            .await;
        // The target behaviour: the surfaced reminder is over a week old, so the agent must relay it as
        // the reminder it set earlier, not as something it has just learned of or just come across. A
        // language judgement — the framing, not any fixed phrase.
        let grounded = judge
            .assess(
                "The reply presents the contract reminder as a standing reminder the agent noted earlier \
                 (for instance, acknowledging it was set a while ago or was for a date now past). It does \
                 NOT present the reminder as fresh news, a new development, or something the agent has \
                 just encountered or just heard about.",
                &material,
            )
            .await;

        vec![
            Verdict::oracle_outcome(
                "the armed wake-up fired and surfaced into a session",
                surfaced,
                "a fired occurrence was raised into a session",
                "no wake-up surfaced after the clock crossed the deadline",
            ),
            verdict_from_judge_outcome(
                "surfaced the due reminder to the user in its reply",
                VerdictKind::Metric,
                delivered,
            ),
            verdict_from_judge_outcome(
                "framed the old reminder as previously set rather than as fresh news",
                VerdictKind::Metric,
                grounded,
            ),
        ]
    }
}
