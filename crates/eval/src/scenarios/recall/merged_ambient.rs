//! Ambient recall over a merged `same_as` identity (issue #92): two `person/*` stubs of one human,
//! operator-merged into a single class, each carrying distinct public content that shares a distinctive
//! lexical stem. A later message from a third party lexically matches both stubs, so the pre-turn
//! ambient pass fires on the whole class. The property under test is deterministic code behaviour, not a
//! model judgement: the pass must collapse the two members to their class primary and surface the
//! identity *once*, under that primary, rather than naming the same person twice on the hint. Before the
//! fix, both stubs survived as separate `AmbientHit`s; after it, the class contributes one hit, keyed to
//! its primary (`graph.class_id`).

use std::{collections::BTreeSet, sync::Arc};

use async_trait::async_trait;
use zuihitsu::{
    EntryId, Event, EventPayload, LinkPosture, LinkSource, MemoryId, MemoryName, RelationName,
    TEST_PLATFORM, Teller, Timestamp, TurnId, Visibility,
};

use crate::{
    analysis,
    context::RUN_START_MS,
    judge::Judge,
    package::{Bar, Category, ScenarioMeta, Verdict},
    scenario::Scenario,
    step::{EvalStep, Turn},
};

/// This module's scenarios.
pub fn scenarios() -> Vec<Arc<dyn Scenario>> {
    vec![Arc::new(MergedIdentitySurfacesOnceInAmbientRecall)]
}

pub struct MergedIdentitySurfacesOnceInAmbientRecall;

/// The two platform-bound stubs of the one supplier, merged into a single `same_as` class. Which handle
/// ends up the class primary is decided by ULID order (the earliest is the default primary), resolved at
/// `steps()` time from the generated ids — the oracle reads the primary back rather than assuming a
/// handle.
const DIRECT_STUB: &str = "person/rowan@direct";
const CHAT_STUB: &str = "person/rowan@chat";

#[async_trait]
impl Scenario for MergedIdentitySurfacesOnceInAmbientRecall {
    fn meta(&self) -> ScenarioMeta {
        ScenarioMeta {
            name: "merged_identity_surfaces_once_in_ambient_recall".to_owned(),
            category: Category::Recall,
            description: "Two person stubs of one human, operator-merged into a single same_as class, \
                          each carrying distinct public content that shares a distinctive lexical stem. \
                          A third party's message lexically matches both stubs, so the pre-turn ambient \
                          recall pass fires on the whole class. The pass must collapse the class to its \
                          primary and surface the identity once, not twice (issue #92)."
                .to_owned(),
            bar: Bar::gating(),
        }
    }

    fn needs_retrieval(&self) -> bool {
        true
    }

    fn steps(&self) -> Vec<EvalStep> {
        // Two clean, platform-bound stubs for one supplier. The ids are sorted so the assignment is
        // deterministic, but which one becomes the class primary is left to the default rule (earliest
        // ULID) — the oracle reads it back from the log rather than assuming, so the test does not bake
        // in the tie-break.
        let mut ids = [MemoryId::generate(), MemoryId::generate()];
        ids.sort();
        let [direct, chat] = ids;

        let mut seed = person_stub(
            direct,
            DIRECT_STUB,
            "Runs the saltmarsh survey out at the north estuary — dawn transects, quadrat counts, the \
             whole routine.",
        );
        seed.extend(person_stub(
            chat,
            CHAT_STUB,
            "Keeps the saltmarsh survey's tide-timing spreadsheet and sorts the crew's dawn logistics.",
        ));
        // Operator-adjudicated merge: the two stubs are one human. `same_as` is seeded at genesis, so the
        // bare link suffices — this is the one path a merge lands (a proposal pends until the operator
        // acts). No primary is pinned, so the class primary is the earliest-ULID default.
        seed.push(EventPayload::link_created(
            direct,
            chat,
            RelationName::SameAs,
            LinkPosture {
                source: LinkSource::Operator,
                told_by: None,
                told_in: None,
                visibility: Visibility::Public,
            },
        ));

        vec![
            EvalStep::SeedEvents(seed),
            // A third party, in another room, asks casually after the survey — a natural conversational
            // beat, not a search probe. The distinctive stem ("saltmarsh survey") matches both merged
            // stubs' content, so the ambient pass fires on the class; the fix must surface it once.
            Turn::new(
                TEST_PLATFORM,
                "harbourside",
                "wren",
                "Hey — any word on how the saltmarsh survey is shaping up? Trying to work out whether to \
                 free up the boat this week.",
            )
            .with_present(&["wren"])
            .into(),
        ]
    }

    async fn assess(&self, events: &[Event], _judge: &Judge) -> Vec<Verdict> {
        // Fixture sanity: both stubs and the merge must exist, or the run tests nothing.
        let direct = analysis::memory_id_named(events, DIRECT_STUB);
        let chat = analysis::memory_id_named(events, CHAT_STUB);
        assert!(
            direct.is_some() && chat.is_some() && analysis::merge_committed(events),
            "the seed must create both stubs and the same_as merge, got direct={direct:?} \
             chat={chat:?} merged={}",
            analysis::merge_committed(events),
        );
        let class: BTreeSet<MemoryId> = [direct, chat].into_iter().flatten().collect();
        // The class primary is the earliest ULID (no designation pinned), which the oracle reads back
        // rather than assuming a handle.
        let primary = class.iter().min().copied();

        // The answering turn — the last agent reply's turn — is where the ambient hint for the question
        // fired, so its `AmbientRecallSurfaced` record carries the hits under test.
        let answering_turn = analysis::agent_replies_with_inbound(events)
            .last()
            .map(|(turn_id, _, _)| *turn_id);
        let class_hits: Vec<MemoryId> = answering_turn
            .map(|turn_id| class_members_in_ambient(events, turn_id, &class))
            .unwrap_or_default();

        // The deduplication invariant: at most one member of the merged class survives as a hit, and any
        // that does is the class primary. Deterministic code behaviour (the ambient pass is a pure
        // function of the seeded log and the fixed inbound text, with no model in the loop), so a strict
        // gate is aligned — a slip is a genuine regression of the collapse, not model variance. It holds
        // vacuously when the class does not surface at all; the metric below guards against that, so a
        // thinned corpus that stops the pass firing shows up as a dropped metric rather than a silent
        // gate.
        let collapses_to_primary =
            class_hits.len() <= 1 && class_hits.iter().all(|hit| Some(*hit) == primary);

        vec![
            Verdict::oracle_outcome(
                "surfaced the merged identity at most once, under its class primary",
                collapses_to_primary,
                "the merged same_as class surfaced as a single ambient hit, keyed to its primary",
                format!(
                    "the merged class surfaced more than once, or under a non-primary member: \
                     hits={class_hits:?} primary={primary:?}"
                ),
            ),
            // Anti-vacuity: the class actually reached the ambient hint. Reported rather than gating,
            // because whether the lexical pass fires depends on the corpus's bm25 separation (which the
            // gate must not be flaky on), while the collapse it guards is deterministic once it does.
            Verdict::metric_outcome(
                "the merged identity reached the ambient hint",
                !class_hits.is_empty(),
                "the ambient pass surfaced the merged class for the answering turn",
                "the ambient pass surfaced no member of the merged class",
            ),
        ]
    }
}

/// A person stub named by its full handle, carrying one public content entry — the shape a merged
/// `same_as` class is built from. Public so the lexical index (public-only) can later surface it.
fn person_stub(id: MemoryId, name: &str, text: &str) -> Vec<EventPayload> {
    vec![
        EventPayload::memory_created(id, MemoryName::new(name)),
        EventPayload::MemoryContentAppended {
            id,
            entry_id: EntryId::generate(),
            asserted_at: Timestamp::from_millis(RUN_START_MS),
            occurred_at: None,
            text: text.to_owned(),
            told_by: Teller::Agent,
            told_in: None,
            visibility: Visibility::Public,
        },
    ]
}

/// The members of `class` that the ambient pass surfaced for `turn_id` — the `AmbientHit` memories on
/// the answering turn's `AmbientRecallSurfaced` record that belong to the merged class.
fn class_members_in_ambient(
    events: &[Event],
    turn_id: TurnId,
    class: &BTreeSet<MemoryId>,
) -> Vec<MemoryId> {
    events
        .iter()
        .filter_map(|event| match &event.payload {
            EventPayload::AmbientRecallSurfaced {
                turn_id: hit_turn,
                hits,
                ..
            } if *hit_turn == turn_id => Some(hits),
            _ => None,
        })
        .flatten()
        .map(|hit| hit.memory)
        .filter(|memory| class.contains(memory))
        .collect()
}
