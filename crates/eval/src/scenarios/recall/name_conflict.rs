//! Recovering from a name collision. When a `create` reaches for a handle that is already taken, the
//! teachable error names the collision and lists the near-matching existing handles, so the agent
//! picks a distinguishing name (`person/dave-chen` versus `person/dave-patel`) rather than colliding
//! repeatedly or minting a near-duplicate. These scenarios build a cluster of same-stem people across
//! separate sessions — the setup that provokes a collision when the agent reaches for the obvious
//! handle — and reward keeping them as distinct memories instead of overwriting one onto another.

use std::sync::Arc;

use async_trait::async_trait;
use zuihitsu::{
    EntryId, Event, EventPayload, MemoryId, MemoryName, Namespace, TEST_PLATFORM, Teller,
    TerminalCause, Timestamp, Visibility,
};

use crate::{
    analysis,
    context::RUN_START_MS,
    judge::Judge,
    package::{Bar, Category, ScenarioMeta, Verdict, VerdictKind, verdict_from_judge_outcome},
    scenario::Scenario,
    step::{EvalStep, Turn},
};

/// This module's scenarios.
pub fn scenarios() -> Vec<Arc<dyn Scenario>> {
    vec![
        Arc::new(DistinguishesCollidingPeople),
        Arc::new(RecoversFromASeededCollision),
    ]
}

/// Three distinct people who share a first name — three Daves — are introduced in separate sessions,
/// each an empty buffer so the earlier handles are not in front of the agent. Recording the second and
/// third pulls the agent toward the obvious `person/dave` handle, which collides; the right recovery is
/// to pick a distinguishing handle for each, leaving three separate memories, not to fold a new Dave
/// onto an existing one (a wrong merge) or to give up after colliding.
pub struct DistinguishesCollidingPeople;

#[async_trait]
impl Scenario for DistinguishesCollidingPeople {
    fn meta(&self) -> ScenarioMeta {
        ScenarioMeta {
            name: "distinguishes_colliding_people".to_owned(),
            category: Category::Recall,
            description: "Three distinct people who share a first name are introduced across separate \
                          sessions. Recording each pulls the agent toward the same obvious handle, which \
                          collides; it should pick a distinguishing handle for each — keeping three \
                          separate memories — rather than overwrite one Dave onto another or stall on \
                          the collision."
                .to_owned(),
            bar: Bar::Metric { threshold: 0.7 },
        }
    }

    fn needs_retrieval(&self) -> bool {
        true
    }

    fn steps(&self) -> Vec<EvalStep> {
        vec![
            // Session 1: the first Dave — the backend lead.
            Turn::new(
                TEST_PLATFORM,
                "general",
                "marcus",
                "Someone to keep track of: Dave — he's our backend lead, been here for years.",
            )
            .into(),
            EvalStep::Settle,
            // Session 2, a different room and an empty buffer: a second, unrelated Dave. Recording him
            // reaches for the same obvious handle as the first, so the create collides.
            Turn::new(
                TEST_PLATFORM,
                "design-crit",
                "erin",
                "Adding someone — a different Dave, Dave on the design team who just started this week. \
                 Not the backend Dave, a new hire.",
            )
            .into(),
            EvalStep::Settle,
            // Session 3, another empty buffer: a third Dave again distinct from the first two.
            Turn::new(
                TEST_PLATFORM,
                "sales-sync",
                "marcus",
                "And yet another one to remember: Dave in sales — closed the big account this quarter. \
                 Different person from the backend lead and the designer, just happens to share the name.",
            )
            .into(),
            EvalStep::Settle,
            // A later room with an empty buffer asks the agent to tell the three apart — answering well
            // rests on their being three distinct memories, not one Dave overwritten by the next.
            Turn::new(
                TEST_PLATFORM,
                "planning",
                "erin",
                "We've got three Daves now and I keep mixing them up — who's who again?",
            )
            .into(),
        ]
    }

    async fn assess(&self, events: &[Event], judge: &Judge) -> Vec<Verdict> {
        // The person memories under the shared stem: exactly the three real people if each collision
        // resolved to a distinguishing handle, fewer if a new Dave was folded onto an existing one (the
        // wrong merge the collision error steers away from), more if a phantom variant was minted.
        let dave_memories: Vec<String> =
            analysis::memories_in_namespace(events, Namespace::Person.prefix())
                .into_iter()
                .filter(|name| name.to_lowercase().contains("dave"))
                .collect();
        let three_distinct = dave_memories.len() == 3;
        let reply = analysis::last_agent_reply(events).unwrap_or_default();
        let judged = judge
            .assess(
                "The reply distinguishes all three Daves as separate people — the backend lead, the \
                 designer (a recent hire), and the one in sales — and does not conflate two of them or \
                 drop one.",
                &format!(
                    "Three different people who share the first name Dave were introduced across \
                     earlier sessions: a long-standing backend lead, a designer who just started, and \
                     one in sales who closed a big account. Asked who the three Daves are, the agent \
                     replied:\n\"{reply}\""
                ),
            )
            .await;

        vec![
            Verdict::metric_outcome(
                "kept the three colliding Daves as distinct memories",
                three_distinct,
                format!("three separate person memories under the shared stem: {dave_memories:?}"),
                format!("the three Daves are not three distinct memories: {dave_memories:?}"),
            ),
            verdict_from_judge_outcome(
                "told the three Daves apart in its reply",
                VerdictKind::Metric,
                judged,
            ),
        ]
    }
}

/// Two same-stem people already exist in the graph — seeded directly, so their handles occupy the
/// obvious stem before the conversation starts — and a turn introduces a third, explicitly distinct
/// Dave. Reaching for the taken `person/dave` handle collides, and the teachable error lists the
/// near-matching existing handles; the rewarded recovery is a distinguishing handle for the new
/// person in at most one collision, never folding him onto an existing Dave and never colliding
/// repeatedly on the same name. Seeding by state rather than by phrasing keeps the steering honest:
/// nothing in the turn tells the agent which handles are taken.
pub struct RecoversFromASeededCollision;

#[async_trait]
impl Scenario for RecoversFromASeededCollision {
    fn meta(&self) -> ScenarioMeta {
        ScenarioMeta {
            name: "recovers_from_a_seeded_collision".to_owned(),
            category: Category::Recall,
            description: "Two same-stem people already occupy the obvious handles when a turn \
                          introduces a third, explicitly distinct Dave. The agent should record him \
                          under a distinguishing handle — recovering from a collision in at most one \
                          attempt if it reaches for a taken name — rather than fold him onto an \
                          existing Dave or collide repeatedly."
                .to_owned(),
            bar: Bar::Metric { threshold: 0.7 },
        }
    }

    fn needs_retrieval(&self) -> bool {
        true
    }

    fn steps(&self) -> Vec<EvalStep> {
        // The occupied stem, set up directly as a synthetic event log rather than by driving the
        // agent: person/dave (the backend lead) and person/dave-ops (a different Dave in ops) exist
        // with committed entries before any conversation opens, so a create that reaches for the
        // obvious handle collides against real state.
        let dave = MemoryId::generate();
        let dave_ops = MemoryId::generate();
        let now = Timestamp::from_millis(RUN_START_MS);
        let seed = vec![
            EventPayload::memory_created(dave, MemoryName::new("person/dave")),
            EventPayload::MemoryContentAppended {
                id: dave,
                entry_id: EntryId::generate(),
                asserted_at: now,
                occurred_at: None,
                text: "The team's backend lead; has been at the company for years.".to_owned(),
                told_by: Teller::Agent,
                told_in: None,
                visibility: Visibility::Public,
            },
            EventPayload::memory_created(dave_ops, MemoryName::new("person/dave-ops")),
            EventPayload::MemoryContentAppended {
                id: dave_ops,
                entry_id: EntryId::generate(),
                asserted_at: now,
                occurred_at: None,
                text: "Runs the ops rotation; a different Dave from the backend lead.".to_owned(),
                told_by: Teller::Agent,
                told_in: None,
                visibility: Visibility::Public,
            },
        ];
        vec![
            EvalStep::SeedEvents(seed),
            // Settle so the seeded entries are described and indexed — the retrieval surface a
            // careful agent would check before creating.
            EvalStep::Settle,
            Turn::new(
                TEST_PLATFORM,
                "sales-sync",
                "marcus",
                "New face on the sales team starting today — also called Dave, no relation to any \
                 Dave we already know. He came over from Aviato and closed a big account in his \
                 first week there. Worth keeping track of him.",
            )
            .into(),
            EvalStep::Settle,
            // A later room with an empty buffer reads the roster back — correct only if the new Dave
            // landed as his own memory beside the two seeded ones.
            Turn::new(
                TEST_PLATFORM,
                "planning",
                "erin",
                "How many Daves are we up to now, and who's who?",
            )
            .into(),
        ]
    }

    async fn assess(&self, events: &[Event], judge: &Judge) -> Vec<Verdict> {
        // Three distinct Daves: the two seeded handles plus one new distinguishing handle. Fewer
        // means the new hire was folded onto an existing Dave; more means a duplicate was minted.
        let dave_memories: Vec<String> =
            analysis::memories_in_namespace(events, Namespace::Person.prefix())
                .into_iter()
                .filter(|name| name.to_lowercase().contains("dave"))
                .collect();
        let three_distinct = dave_memories.len() == 3;
        // The collision-recovery discipline: reaching for a taken handle at most once. Zero
        // collisions (the agent checked first, or guessed a free handle) passes; one collision
        // followed by a distinguishing retry passes; hammering the same taken name does not.
        let collisions = name_collision_count(events);
        let recovered = collisions <= 1;
        let reply = analysis::last_agent_reply(events).unwrap_or_default();
        let judged = judge
            .assess(
                "The reply counts three Daves and tells them apart — the backend lead, the one in \
                 ops, and the new sales hire from Aviato — without conflating any two of them.",
                &format!(
                    "Two people called Dave were already on record (the backend lead, and a \
                     different Dave in ops), then a third — a new sales hire from Aviato — was \
                     introduced. Asked how many Daves there are and who's who, the agent \
                     replied:\n\"{reply}\""
                ),
            )
            .await;

        vec![
            Verdict::metric_outcome(
                "recorded the new Dave as a third distinct memory",
                three_distinct,
                format!("three separate person memories under the stem: {dave_memories:?}"),
                format!("the new Dave did not land as a third distinct memory: {dave_memories:?}"),
            ),
            Verdict::metric_outcome(
                "recovered from any name collision in at most one attempt",
                recovered,
                format!("{collisions} collision(s) — at most one create reached a taken name"),
                format!("collided {collisions} times — kept reaching for taken names"),
            ),
            verdict_from_judge_outcome(
                "counted and distinguished all three Daves in its reply",
                VerdictKind::Metric,
                judged,
            ),
        ]
    }
}

/// The number of Lua blocks in the run that terminated on a name or tag collision — a terminal cause
/// carrying the "already exists" teachable error. The collision-recovery metric reads this: at most
/// one is a clean recovery, more is the repeated-collision failure the suggestions exist to prevent.
fn name_collision_count(events: &[Event]) -> usize {
    events
        .iter()
        .filter(|event| {
            matches!(
                &event.payload,
                EventPayload::LuaExecuted {
                    terminal_cause: Some(TerminalCause::Error(message)),
                    ..
                } if message.contains("already exists")
            )
        })
        .count()
}
