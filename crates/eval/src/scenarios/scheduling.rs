//! Capturing and firing a recurring reminder (migrated from
//! `real_model_keeps_and_surfaces_a_recurring_reminder`). A quality metric on the scaffold-shaped Right
//! Thing: from an un-coached request, record the reminder as a recurring occurrence on an `event/`
//! memory (which then fires a week later when the clock advances and a fresh session opens).

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

/// This module's scenarios.
pub fn scenarios() -> Vec<Arc<dyn Scenario>> {
    vec![Arc::new(RecurringReminder), Arc::new(RecurringEmission)]
}

/// Eight days — past the first weekly instance of a reminder recorded at the run's start.
const EIGHT_DAYS_MS: i64 = 8 * 24 * 60 * 60 * 1_000;

pub struct RecurringReminder;

#[async_trait]
impl Scenario for RecurringReminder {
    fn meta(&self) -> ScenarioMeta {
        ScenarioMeta {
            name: "recurring_reminder".to_owned(),
            category: Category::Scheduling,
            description: "From a natural, un-coached request, the agent records a recurring reminder \
                          as an event/ memory; advancing past the first instance and opening a fresh \
                          session fires it."
                .to_owned(),
            bar: Bar::Metric { threshold: 0.6 },
        }
    }

    async fn run(&self, ctx: &RunContext) -> Result<(), EvalError> {
        ctx.turn(Turn::new(
            "discord",
            "team-room",
            "phil",
            "Can you remind me about our team standup? It's every Monday.",
        ))
        .await?;
        // Advance past the first weekly instance, then a fresh-session turn fires the wake-up.
        ctx.advance(EIGHT_DAYS_MS);
        ctx.turn(Turn::new(
            "discord",
            "team-room",
            "phil",
            "Morning! Anything on my plate I should know about?",
        ))
        .await?;
        Ok(())
    }

    async fn assess(&self, events: &[Event], judge: &Judge) -> Vec<Verdict> {
        let recurring = analysis::recurring_memory_names(events);
        let recorded = !recurring.is_empty();
        let on_event = recurring.iter().any(|name| name.starts_with("event/"));
        let surfaced = analysis::scheduled_item_surfaced(events);

        // The end-to-end check: a week on, asked what's on his plate, did the reply actually tell phil
        // about the standup? Structural events show the wake-up fired; only the reply shows it landed.
        let reply = analysis::last_agent_reply(events).unwrap_or_default();
        let delivered = judge
            .assess(
                "The reply tells the user about the standup it was earlier asked to remind them of — \
                 naming the standup and/or that it falls on Monday.",
                &format!(
                    "Earlier, the agent was asked to set a recurring reminder about a team standup \
                     held every Monday. A week later, asked \"anything on my plate I should know \
                     about?\", the agent replied:\n\"{reply}\""
                ),
            )
            .await;

        vec![
            Verdict::metric_outcome(
                "recorded the reminder as a recurring occurrence",
                recorded,
                format!("recurring memories: {recurring:?}"),
                "did not record a recurring occurrence",
            ),
            Verdict::metric_outcome(
                "filed the schedule on an event/ memory",
                on_event,
                "the recurring memory is in the event/ namespace",
                "the recurring occurrence is not on an event/ memory",
            ),
            Verdict::metric_outcome(
                "the wake-up fired and surfaced into a session",
                surfaced,
                "a fired occurrence was raised into a session",
                "no wake-up surfaced after the clock advanced",
            ),
            Verdict::from_judge_outcome(
                "surfaced the reminder to the user in its reply",
                VerdictKind::Metric,
                delivered,
            ),
        ]
    }
}

/// Emitting a recurring occurrence from a plainly recurring phrase (migrated from `eval_recurrence.rs`).
/// The deterministic tests prove a `Recurring` occurrence stores and reads back; this asks whether the
/// model *emits* one for "every Tuesday" rather than flattening it to a single day. A tracked rate.
pub struct RecurringEmission;

#[async_trait]
impl Scenario for RecurringEmission {
    fn meta(&self) -> ScenarioMeta {
        ScenarioMeta {
            name: "emits_a_recurring_occurrence".to_owned(),
            category: Category::Scheduling,
            description: "From a plainly recurring phrase (\"every Tuesday\"), the agent emits a \
                          recurring temporal reference rather than flattening it to a single day."
                .to_owned(),
            bar: Bar::Metric { threshold: 0.6 },
        }
    }

    async fn run(&self, ctx: &RunContext) -> Result<(), EvalError> {
        ctx.turn(Turn::new(
            "discord",
            "leads",
            "dave",
            "Please remember that I have a team standup every Tuesday at 9am.",
        ))
        .await?;
        Ok(())
    }

    async fn assess(&self, events: &[Event], _judge: &Judge) -> Vec<Verdict> {
        let emitted = analysis::has_recurring_occurrence(events);
        vec![Verdict::metric_outcome(
            "emitted a recurring occurrence",
            emitted,
            "recorded a recurring temporal reference",
            "flattened the recurrence to a single day, or recorded none",
        )]
    }
}
