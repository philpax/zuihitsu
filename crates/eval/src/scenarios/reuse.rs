//! Reusing existing resources instead of recreating them — graph hygiene under update. When an entity
//! already exists and new information about it arrives in a *later* session (an empty buffer, so the
//! handle is not in front of the agent), the right move is to search for the existing memory and update
//! it in place — append, supersede, or link — not mint a second memory for the same thing, which
//! fragments what is known across duplicates. The single-session update is easy (the buffer still holds
//! the handle); these isolate the hard cross-session case, where reuse rests on retrieval.

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
    vec![
        Arc::new(UpdatesAnExistingEvent),
        Arc::new(AddsToAnExistingPerson),
        Arc::new(LinksExistingMemories),
    ]
}

/// An event is put on the calendar in one session, then in a later session — a different room, an empty
/// buffer — its date changes. The agent should find the existing event and update it, not create a
/// second event memory for the same launch.
pub struct UpdatesAnExistingEvent;

#[async_trait]
impl Scenario for UpdatesAnExistingEvent {
    fn meta(&self) -> ScenarioMeta {
        ScenarioMeta {
            name: "updates_an_existing_event".to_owned(),
            category: Category::Recall,
            description: "An event is calendared in one session, then its date changes in a later \
                          session (a different room, an empty buffer). The agent should search out the \
                          existing event and update it in place, not mint a second event memory for the \
                          same launch."
                .to_owned(),
            bar: Bar::Metric { threshold: 0.7 },
        }
    }

    fn needs_retrieval(&self) -> bool {
        true
    }

    async fn run(&self, ctx: &RunContext) -> Result<(), EvalError> {
        ctx.turn(Turn::new(
            "discord",
            "planning",
            "phil",
            "Let's get the product launch on the calendar — it's set for the 15th of March.",
        ))
        .await?;
        ctx.describe_catch_up().await?;
        ctx.index_catch_up().await?;
        // A later session, a different room, an empty buffer: the event handle is not in front of the
        // agent, so updating it in place requires finding it first.
        ctx.turn(Turn::new(
            "discord",
            "standup",
            "phil",
            "Update on the product launch — it's moved to the 22nd of March now.",
        ))
        .await?;
        ctx.describe_catch_up().await?;
        ctx.index_catch_up().await?;
        ctx.turn(Turn::new(
            "discord",
            "hallway",
            "erin",
            "Remind me — when's the product launch?",
        ))
        .await?;
        Ok(())
    }

    async fn assess(&self, events: &[Event], judge: &Judge) -> Vec<Verdict> {
        let event_memories = analysis::memories_in_namespace(events, "event/");
        let single = event_memories.len() == 1;
        let reply = analysis::last_agent_reply(events).unwrap_or_default();
        let judged = judge
            .assess(
                "The reply gives the launch date as the 22nd of March (the updated date), not the 15th.",
                &format!(
                    "A product launch was first calendared for the 15th of March, then in a later \
                     session moved to the 22nd. Asked when the launch is, the agent replied:\n\"{reply}\""
                ),
            )
            .await;

        vec![
            Verdict::metric_outcome(
                "reused the existing event rather than creating a duplicate",
                single,
                format!("one event memory holds the launch: {event_memories:?}"),
                format!("the launch is split across event memories: {event_memories:?}"),
            ),
            Verdict::from_judge_outcome(
                "answered with the current launch date",
                VerdictKind::Metric,
                judged,
            ),
        ]
    }
}

/// A person is recorded in one session, then a new fact about them arrives in a later session. The agent
/// should find that person's existing memory and append to it, not start a second stub for the same
/// person.
pub struct AddsToAnExistingPerson;

#[async_trait]
impl Scenario for AddsToAnExistingPerson {
    fn meta(&self) -> ScenarioMeta {
        ScenarioMeta {
            name: "adds_to_an_existing_person".to_owned(),
            category: Category::Recall,
            description: "A person is recorded in one session, then a new fact about them arrives in a \
                          later session (a different room, an empty buffer). The agent should find their \
                          existing memory and append to it, not start a second stub for the same person."
                .to_owned(),
            bar: Bar::Metric { threshold: 0.7 },
        }
    }

    fn needs_retrieval(&self) -> bool {
        true
    }

    async fn run(&self, ctx: &RunContext) -> Result<(), EvalError> {
        ctx.turn(Turn::new(
            "discord",
            "general",
            "phil",
            "Someone to keep track of: Dave — he's a product designer at Hooli.",
        ))
        .await?;
        ctx.describe_catch_up().await?;
        ctx.index_catch_up().await?;
        // A later session, an empty buffer: appending the new fact to Dave requires finding him first.
        ctx.turn(Turn::new(
            "discord",
            "standup",
            "phil",
            "Heads up — Dave just got promoted to engineering lead.",
        ))
        .await?;
        ctx.describe_catch_up().await?;
        ctx.index_catch_up().await?;
        ctx.turn(Turn::new(
            "discord",
            "hallway",
            "erin",
            "What's Dave's role these days?",
        ))
        .await?;
        Ok(())
    }

    async fn assess(&self, events: &[Event], judge: &Judge) -> Vec<Verdict> {
        // Memories that name Dave — exactly one if the later fact accreted onto his existing memory, more
        // if a second stub was started (the speakers' own memories do not name Dave, so they do not count).
        let dave_memories: Vec<String> = analysis::memory_names(events)
            .into_values()
            .filter(|name| name.to_lowercase().contains("dave"))
            .collect();
        let single = dave_memories.len() == 1;
        let reply = analysis::last_agent_reply(events).unwrap_or_default();
        let judged = judge
            .assess(
                "The reply says Dave's current role is engineering lead (his role after the promotion), \
                 not only product designer.",
                &format!(
                    "Dave was first recorded as a product designer at Hooli, then in a later session \
                     promoted to engineering lead. Asked his role these days, the agent replied:\n\
                     \"{reply}\""
                ),
            )
            .await;

        vec![
            Verdict::metric_outcome(
                "accreted onto the existing person rather than starting a second stub",
                single,
                format!("one memory holds Dave: {dave_memories:?}"),
                format!("Dave is split across memories: {dave_memories:?}"),
            ),
            Verdict::from_judge_outcome(
                "answered with Dave's current role",
                VerdictKind::Metric,
                judged,
            ),
        ]
    }
}

/// Two people are recorded in separate earlier sessions, then a later session says they know each other.
/// The agent should retrieve both existing memories and link them, not create fresh stubs to link — a
/// link between duplicates connects nothing the rest of the system already knows.
pub struct LinksExistingMemories;

#[async_trait]
impl Scenario for LinksExistingMemories {
    fn meta(&self) -> ScenarioMeta {
        ScenarioMeta {
            name: "links_existing_memories".to_owned(),
            category: Category::Relations,
            description: "Two people are recorded in separate earlier sessions, then a later session \
                          says they know each other. The agent should retrieve both existing memories \
                          and link them, not mint fresh stubs to link."
                .to_owned(),
            bar: Bar::Metric { threshold: 0.6 },
        }
    }

    fn needs_retrieval(&self) -> bool {
        true
    }

    async fn run(&self, ctx: &RunContext) -> Result<(), EvalError> {
        ctx.turn(Turn::new(
            "discord",
            "general",
            "phil",
            "Someone to remember: Dave, a product designer at Hooli.",
        ))
        .await?;
        ctx.describe_catch_up().await?;
        ctx.index_catch_up().await?;
        // A separate session for Erin — her handle is recorded with Dave's not in the buffer.
        ctx.turn(Turn::new(
            "discord",
            "intros",
            "phil",
            "Another to remember: Erin, a product manager on the platform team.",
        ))
        .await?;
        ctx.describe_catch_up().await?;
        ctx.index_catch_up().await?;
        // A third session asserts the relationship: linking requires retrieving both existing people.
        ctx.turn(Turn::new(
            "discord",
            "team-room",
            "phil",
            "By the way, Dave and Erin know each other well — they've worked together for years.",
        ))
        .await?;
        Ok(())
    }

    async fn assess(&self, events: &[Event], _judge: &Judge) -> Vec<Verdict> {
        let names = analysis::memory_names(events);
        let dave = names
            .values()
            .filter(|name| name.to_lowercase().contains("dave"))
            .count();
        let erin = names
            .values()
            .filter(|name| name.to_lowercase().contains("erin"))
            .count();
        let no_duplicates = dave == 1 && erin == 1;
        let linked = analysis::link_created_with(events, "knows");

        vec![Verdict::metric_outcome(
            "linked the two existing people without minting duplicate stubs",
            no_duplicates && linked,
            "one Dave memory and one Erin memory, joined by a knows link",
            format!(
                "duplicated a stub or did not link ({dave} Dave, {erin} Erin, linked={linked})"
            ),
        )]
    }
}
