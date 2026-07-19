//! A `[mem:<id>]` reference resolves in ambient recall (issue #81): a person stub carries public
//! identity content, and a third party's message embeds that person's memory-reference token — the shape
//! a Discord connector produces when it splices a projected @mention. The pre-turn ambient pass must
//! decode the token to the person's handle so the reference is legible: the agent reads who the token
//! points at and speaks about them by name rather than treating the token as opaque.
//!
//! Two properties. The resolution is deterministic code (the ambient pass is a pure function of the
//! seeded log and the fixed inbound text, with no model in the loop), so the gate reads the answering
//! turn's `AmbientRecallSurfaced` record and asserts it carries the handle the token resolved to. The
//! reply's use of the name is a model judgement, reported as a lenient structural metric rather than
//! gated.

use std::sync::Arc;

use async_trait::async_trait;
use zuihitsu::{
    EntryId, Event, EventPayload, MemoryId, MemoryName, TEST_PLATFORM, Teller, TurnId, Visibility,
    mem_ref,
};

use crate::{
    analysis,
    context::run_start,
    judge::Judge,
    package::{Bar, Category, ScenarioMeta, Verdict},
    scenario::Scenario,
    step::{EvalStep, Turn},
};

/// This module's scenarios.
pub fn scenarios() -> Vec<Arc<dyn Scenario>> {
    vec![Arc::new(MentionMemrefResolvesInAmbientRecall)]
}

pub struct MentionMemrefResolvesInAmbientRecall;

/// The referenced person's handle — a platform-bound `person/*` stub, the shape a projected mention
/// mints (`person/<user_id>@<platform>`). The stem is distinctive so the reply's use of the name reads
/// unambiguously.
const ROWAN_STUB: &str = "person/rowan@chat";

#[async_trait]
impl Scenario for MentionMemrefResolvesInAmbientRecall {
    fn meta(&self) -> ScenarioMeta {
        ScenarioMeta {
            name: "mention_memref_resolves_in_ambient_recall".to_owned(),
            category: Category::Recall,
            description: "A person stub carries public identity content, and a third party's message \
                          embeds that person's [mem:<id>] token — the shape a Discord connector produces \
                          when it splices a projected @mention. The pre-turn ambient pass must decode the \
                          token to the person's handle so the reference is legible, and the reply should \
                          speak about the right person by name rather than treating the token as opaque \
                          (issue #81)."
                .to_owned(),
            bar: Bar::gating(),
        }
    }

    fn needs_retrieval(&self) -> bool {
        true
    }

    fn steps(&self) -> Vec<EvalStep> {
        // A clean, platform-bound stub for the referenced person, carrying public identity content so the
        // reply can speak about them. The id is generated here, so the inbound can embed the person's
        // canonical `[mem:<id>]` token directly — exactly the token a connector splices for a mention.
        let rowan = MemoryId::generate();
        let seed = person_stub(
            rowan,
            ROWAN_STUB,
            "Rowan runs the harbourside boat crew and is usually around midweek.",
        );

        vec![
            EvalStep::SeedEvents(seed),
            // A third party asks casually after Rowan, referencing them by the memory token a spliced
            // mention would carry — a natural conversational beat, not a search probe. The ambient pass
            // must decode the token so the agent knows who is being asked about.
            Turn::new(
                TEST_PLATFORM,
                "harbourside",
                "wren",
                format!(
                    "Hey — has {} been around this week? Trying to sort out the boat roster.",
                    mem_ref::construct(rowan)
                ),
            )
            .with_present(&["wren"])
            .into(),
        ]
    }

    async fn assess(&self, events: &[Event], _judge: &Judge) -> Vec<Verdict> {
        // Fixture sanity: the referenced stub must exist, or the run tests nothing.
        assert!(
            analysis::memory_id_named(events, ROWAN_STUB).is_some(),
            "the seed must create the referenced person stub {ROWAN_STUB}",
        );

        // The answering turn — the last agent reply — is where the ambient hint for the question fired,
        // so its `AmbientRecallSurfaced` record carries the resolution under test.
        let answering = analysis::agent_replies_with_inbound(events).last().copied();
        let answering_turn = answering.map(|(turn_id, _, _)| turn_id);
        let reply = answering.map(|(_, _, reply)| reply).unwrap_or_default();

        // The gate: the ambient hint for the answering turn decoded the token to the person's handle.
        // Deterministic code behaviour (the resolution is a pure function of the seeded log and the fixed
        // inbound token), so a strict gate is aligned — a slip is a genuine regression of the resolution,
        // not model variance.
        let resolved_in_hint = answering_turn
            .map(|turn_id| ambient_text_for(events, turn_id).contains(ROWAN_STUB))
            .unwrap_or(false);

        // The metric: the reply speaks about the right person by name and does not parrot the raw token.
        // A model judgement, so reported rather than gated — and kept structural and lenient (the stem,
        // not a pinned phrasing) so it does not punish natural wording.
        let names_the_person = reply.to_lowercase().contains("rowan") && !reply.contains("[mem:");

        vec![
            Verdict::oracle_outcome(
                "the ambient hint decoded the memory token to the person's handle",
                resolved_in_hint,
                "the answering turn's ambient hint resolved the [mem:<id>] token to person/rowan@chat",
                "the answering turn's ambient hint did not carry the token's resolution to the handle",
            ),
            Verdict::metric_outcome(
                "the reply speaks about the right person by name, not the opaque token",
                names_the_person,
                "the reply named Rowan rather than echoing the raw memory token",
                "the reply did not name the person, or echoed the raw [mem:<id>] token",
            ),
        ]
    }
}

/// A person stub named by its full handle, carrying one public content entry — the shape a projected
/// mention mints. Public so the identity is readable when the agent answers.
fn person_stub(id: MemoryId, name: &str, text: &str) -> Vec<EventPayload> {
    vec![
        EventPayload::memory_created(id, MemoryName::new(name)),
        EventPayload::MemoryContentAppended {
            id,
            entry_id: EntryId::generate(),
            asserted_at: run_start(),
            occurred_at: None,
            text: text.to_owned(),
            told_by: Teller::Agent,
            told_in: None,
            visibility: Visibility::Public,
        },
    ]
}

/// The concatenated text of every `AmbientRecallSurfaced` record for `turn_id` — the rendered hint the
/// answering turn read, where a resolved memory reference leaves its token-to-handle line.
fn ambient_text_for(events: &[Event], turn_id: TurnId) -> String {
    events
        .iter()
        .filter_map(|event| match &event.payload {
            EventPayload::AmbientRecallSurfaced {
                turn_id: hit_turn,
                text,
                ..
            } if *hit_turn == turn_id => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}
