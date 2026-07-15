//! Reusing existing resources instead of recreating them — graph hygiene under update. When an entity
//! already exists and new information about it arrives in a *later* session (an empty buffer, so the
//! handle is not in front of the agent), the right move is to search for the existing memory and update
//! it in place — append, supersede, or link — not mint a second memory for the same thing, which
//! fragments what is known across duplicates. The single-session update is easy (the buffer still holds
//! the handle); these isolate the hard cross-session case, where reuse rests on retrieval.

use std::sync::Arc;

use async_trait::async_trait;
use zuihitsu::{Event, Namespace, TEST_PLATFORM};

use crate::{
    analysis,
    judge::Judge,
    package::{Bar, Category, ScenarioMeta, Verdict, VerdictKind, verdict_from_judge_outcome},
    scenario::Scenario,
    step::{EvalStep, Turn},
};

/// This module's scenarios.
pub fn scenarios() -> Vec<Arc<dyn Scenario>> {
    vec![
        Arc::new(UpdatesAnExistingEvent),
        Arc::new(AddsToAnExistingPerson),
        Arc::new(LinksExistingMemories),
        Arc::new(DiscoversHandlesByStem),
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

    fn steps(&self) -> Vec<EvalStep> {
        vec![
            Turn::new(
                TEST_PLATFORM,
                "planning",
                "marcus",
                "Let's get the product launch on the calendar — it's set for the 15th of March.",
            )
            .into(),
            EvalStep::Settle,
            // A later session, a different room, an empty buffer: the event handle is not in front of the
            // agent, so updating it in place requires finding it first.
            Turn::new(
                TEST_PLATFORM,
                "standup",
                "marcus",
                "Update on the product launch — it's moved to the 22nd of March now.",
            )
            .into(),
            EvalStep::Settle,
            Turn::new(
                TEST_PLATFORM,
                "hallway",
                "erin",
                "Remind me — when's the product launch?",
            )
            .into(),
        ]
    }

    async fn assess(&self, events: &[Event], judge: &Judge) -> Vec<Verdict> {
        let event_memories = analysis::memories_in_namespace(events, Namespace::Event.prefix());
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
            Verdict::oracle_outcome(
                "reused the existing event rather than creating a duplicate",
                single,
                format!("one event memory holds the launch: {event_memories:?}"),
                format!("the launch is split across event memories: {event_memories:?}"),
            ),
            verdict_from_judge_outcome(
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

    fn steps(&self) -> Vec<EvalStep> {
        vec![
            Turn::new(
                TEST_PLATFORM,
                "general",
                "marcus",
                "Someone to keep track of: Dave — he's a product designer at Hooli.",
            )
            .into(),
            EvalStep::Settle,
            // A later session, an empty buffer: appending the new fact to Dave requires finding him first.
            Turn::new(
                TEST_PLATFORM,
                "standup",
                "marcus",
                "Heads up — Dave just got promoted to engineering lead.",
            )
            .into(),
            EvalStep::Settle,
            Turn::new(
                TEST_PLATFORM,
                "hallway",
                "erin",
                "What's Dave's role these days?",
            )
            .into(),
        ]
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
            Verdict::oracle_outcome(
                "accreted onto the existing person rather than starting a second stub",
                single,
                format!("one memory holds Dave: {dave_memories:?}"),
                format!("Dave is split across memories: {dave_memories:?}"),
            ),
            verdict_from_judge_outcome(
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
            category: Category::Recall,
            description: "Two people are recorded in separate earlier sessions, then a later session \
                          says they know each other. The agent should retrieve both existing memories \
                          and link them, not mint fresh stubs to link."
                .to_owned(),
            bar: Bar::gating(),
        }
    }

    fn needs_retrieval(&self) -> bool {
        true
    }

    fn steps(&self) -> Vec<EvalStep> {
        vec![
            Turn::new(
                TEST_PLATFORM,
                "general",
                "marcus",
                "Someone to remember: Dave, a product designer at Hooli.",
            )
            .into(),
            EvalStep::Settle,
            // A separate session for Erin — her handle is recorded with Dave's not in the buffer.
            Turn::new(
                TEST_PLATFORM,
                "intros",
                "marcus",
                "Another to remember: Erin, a product manager on the platform team.",
            )
            .into(),
            EvalStep::Settle,
            // A third session asserts the relationship: linking requires retrieving both existing people.
            Turn::new(
                TEST_PLATFORM,
                "team-room",
                "marcus",
                "By the way, Dave and Erin know each other well — they've worked together for years.",
            )
            .into(),
        ]
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

        vec![Verdict::oracle_outcome(
            "linked the two existing people without minting duplicate stubs",
            no_duplicates && linked,
            "one Dave memory and one Erin memory, joined by a knows link",
            format!(
                "duplicated a stub or did not link ({dave} Dave, {erin} Erin, linked={linked})"
            ),
        )]
    }
}

/// Two people whose handles share a stem — David and Davina — are recorded in separate earlier
/// sessions. A later session asks, without naming either, who "the Davs" on the team are. The referent
/// is not a single guessable handle but a *family* of them, so the recognizing move is to discover the
/// existing handles under the stem (memory.list("person/dav"), or a search that surfaces both) and
/// answer from what is there — not to guess a handle like person/dave and mint a phantom variant that
/// belongs to neither real person.
pub struct DiscoversHandlesByStem;

#[async_trait]
impl Scenario for DiscoversHandlesByStem {
    fn meta(&self) -> ScenarioMeta {
        ScenarioMeta {
            name: "discovers_handles_by_stem".to_owned(),
            category: Category::Recall,
            description: "Two people who share a name stem — David and Davina — are recorded in \
                          separate earlier sessions. A later session asks who \"the Davs\" are without \
                          naming either. The agent should discover the existing handles under the stem \
                          and answer from them, not guess a handle and mint a phantom variant."
                .to_owned(),
            bar: Bar::Metric { threshold: 0.7 },
        }
    }

    fn needs_retrieval(&self) -> bool {
        true
    }

    fn steps(&self) -> Vec<EvalStep> {
        vec![
            // Session 1: David recorded.
            Turn::new(
                TEST_PLATFORM,
                "general",
                "marcus",
                "Someone to keep track of: David — he's our new backend lead, came over from Hooli.",
            )
            .into(),
            EvalStep::Settle,
            // Unrelated chatter, a different room — noise between the two introductions.
            Turn::new(
                TEST_PLATFORM,
                "random",
                "erin",
                "Whoever's been restocking the good coffee in the kitchen — you're doing the lord's work.",
            )
            .into(),
            EvalStep::Settle,
            // Session 2: Davina recorded — a different person, the same stem.
            Turn::new(
                TEST_PLATFORM,
                "intros",
                "marcus",
                "Another one for the roster: Davina — she's leading the new design-system work.",
            )
            .into(),
            EvalStep::Settle,
            // Session 3: an empty buffer, neither handle in front of the agent. Answering "the Davs"
            // without naming them rewards enumerating the stem over guessing a single handle.
            Turn::new(
                TEST_PLATFORM,
                "planning",
                "marcus",
                "Quick one — who are all the Davs on the team again? I always get them mixed up.",
            )
            .into(),
        ]
    }

    async fn assess(&self, events: &[Event], judge: &Judge) -> Vec<Verdict> {
        // The person memories under the shared stem: exactly the two real people if no phantom variant
        // was minted, more if a guessed handle (person/dave, …) started a third.
        let dav_memories: Vec<String> = analysis::memories_in_namespace(events, "person/")
            .into_iter()
            .filter(|name| name.to_lowercase().contains("dav"))
            .collect();
        let no_phantom = dav_memories.len() == 2;
        let reply = analysis::last_agent_reply(events).unwrap_or_default();
        let judged = judge
            .assess(
                "The reply names both David and Davina as the two distinct people (David the backend \
                 lead, Davina leading the design-system work) and does not invent a third person such \
                 as a 'Dave' who was never recorded.",
                &format!(
                    "Two teammates who share a name stem were recorded earlier — David, the backend \
                     lead, and Davina, leading the design-system work. Asked who all \"the Davs\" on \
                     the team are, the agent replied:\n\"{reply}\""
                ),
            )
            .await;

        vec![
            Verdict::metric_outcome(
                "did not mint a phantom handle variant for the shared stem",
                no_phantom,
                format!("exactly the two real people under the stem: {dav_memories:?}"),
                format!(
                    "a phantom or duplicate variant was created under the stem: {dav_memories:?}"
                ),
            ),
            verdict_from_judge_outcome(
                "named both real people under the stem, inventing none",
                VerdictKind::Metric,
                judged,
            ),
        ]
    }
}
