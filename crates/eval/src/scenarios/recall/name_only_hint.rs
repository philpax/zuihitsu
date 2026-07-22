//! A name-only ambient hint renders without a fabricated quote (issue #102): a memory with a
//! distinctive name but no public content or description generates an FTS hit whose snippet is the
//! name itself — the degenerate name-as-snippet pattern. The render fix produces a bare handle line
//! (`- topic/zephyr`) rather than a fabricated quote (`- topic/zephyr — "topic/zephyr"`).
//!
//! The property under test is deterministic code behaviour, not a model judgement: the ambient pass
//! is a pure function of the seeded log and the fixed inbound text, with no model in the loop. The
//! oracle reads the answering turn's `AmbientRecallSurfaced` record's `text` field — the exact
//! rendered hint stored verbatim — and checks that the bare handle is present and the fabricated
//! quote is absent. An anti-vacuity metric guards against the gate passing vacuously when the
//! ambient pass does not fire.

use std::sync::Arc;

use async_trait::async_trait;
use zuihitsu::{
    EntryId, Event, EventPayload, MemoryId, Namespace, TEST_PLATFORM, Teller, TurnId, Visibility,
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
    vec![Arc::new(NameOnlyHintRendersWithoutFabricatedQuote)]
}

pub struct NameOnlyHintRendersWithoutFabricatedQuote;

/// The memory's distinctive name — a topic with no content or description, so the only FTS match is
/// on the name column, and the snippet is the name itself.
const ZEPHYR: &str = "topic/zephyr";

#[async_trait]
impl Scenario for NameOnlyHintRendersWithoutFabricatedQuote {
    fn meta(&self) -> ScenarioMeta {
        ScenarioMeta {
            name: "name_only_hint_renders_without_fabricated_quote".to_owned(),
            category: Category::Recall,
            description: "A topic memory with a distinctive name but no content or description. \
                          A message lexically matching the name generates an FTS hit whose snippet \
                          is the name itself. The render fix must produce a bare handle line, not \
                          a fabricated quote (issue #102)."
                .to_owned(),
            bar: Bar::gating(),
        }
    }

    fn needs_retrieval(&self) -> bool {
        true
    }

    fn steps(&self) -> Vec<EvalStep> {
        let zephyr = MemoryId::generate();
        // The target memory: a topic named `topic/zephyr` with no content and no description — the
        // honest name-only case. The FTS index carries the name token, so "zephyr" matches the name
        // column alone, and the snippet is the name.
        let mut seed = vec![EventPayload::memory_created(
            zephyr,
            Namespace::Topic.with_name("zephyr"),
        )];
        // A dozen filler topic memories with unrelated public content, so the FTS index carries a
        // realistic corpus and the name match has a non-trivial BM25 score. With `unicode61`
        // tokenization, "zephyr" matches the name token of `topic/zephyr` (a 2-token field). The IDF
        // term dominates in a 13-document corpus for a term appearing in exactly one document.
        for i in 0..12 {
            let filler = MemoryId::generate();
            seed.push(EventPayload::memory_created(
                filler,
                Namespace::Topic.with_name(format!("filler{i}")),
            ));
            seed.push(EventPayload::MemoryContentAppended {
                id: filler,
                entry_id: EntryId::generate(),
                asserted_at: run_start(),
                occurred_at: None,
                text: format!("Unrelated note {i} about weather, lunch, and travel plans."),
                told_by: Teller::Agent,
                told_in: None,
                visibility: Visibility::Public,
            });
        }

        vec![
            EvalStep::SeedEvents(seed),
            // A third party asks about zephyr — the distinctive word matches the name token of the
            // target memory, so the ambient pass fires and the snippet is the name itself.
            Turn::new(
                TEST_PLATFORM,
                "watercooler",
                "wren",
                "What do you know about zephyr?",
            )
            .with_present(&["wren"])
            .into(),
        ]
    }

    async fn assess(&self, events: &[Event], _judge: &Judge) -> Vec<Verdict> {
        // Fixture sanity: the target memory must exist, or the run tests nothing.
        let zephyr_id = analysis::memory_id_named(events, ZEPHYR);
        assert!(
            zephyr_id.is_some(),
            "the seed must create the target memory {ZEPHYR}",
        );

        // The answering turn — the last agent reply's turn — is where the ambient hint for the
        // question fired, so its `AmbientRecallSurfaced` record carries the rendered text under test.
        let answering_turn = analysis::agent_replies_with_inbound(events)
            .last()
            .map(|(turn_id, _, _)| *turn_id);
        let text = answering_turn
            .map(|turn_id| ambient_text_for(events, turn_id))
            .unwrap_or_default();

        // The gate: the rendered hint contains the bare handle (`- topic/zephyr`) and does not
        // contain the fabricated quote (`"topic/zephyr"`). Both conditions must hold — a strict gate
        // is aligned, since the render function is deterministic code.
        let bare_handle_present = text.contains("- topic/zephyr");
        let fabricated_quote_absent = !text.contains("\"topic/zephyr\"");
        let renders_correctly = bare_handle_present && fabricated_quote_absent;

        // Anti-vacuity: the target memory reached the ambient hint. Reported rather than gated,
        // because whether the lexical pass fires depends on the corpus's BM25 separation, while the
        // render it guards is deterministic once it does.
        let surfaced = answering_turn.is_some_and(|turn_id| {
            zephyr_id.is_some_and(|id| ambient_hits_include(events, turn_id, id))
        });

        vec![
            Verdict::oracle_outcome(
                "the name-only hint rendered as a bare handle, not a fabricated quote",
                renders_correctly,
                "the answering turn's ambient hint carried the bare handle - topic/zephyr, with no \
                 fabricated quote",
                format!(
                    "the ambient hint did not render the bare handle correctly: \
                     bare_handle_present={bare_handle_present} \
                     fabricated_quote_absent={fabricated_quote_absent} text={text:?}"
                ),
            ),
            Verdict::metric_outcome(
                "the name-only memory reached the ambient hint",
                surfaced,
                "the ambient pass surfaced the topic/zephyr memory for the answering turn",
                "the ambient pass did not surface the topic/zephyr memory",
            ),
        ]
    }
}

/// The concatenated text of every `AmbientRecallSurfaced` record for `turn_id` — the rendered hint
/// the model read, stored verbatim.
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

/// Whether any `AmbientRecallSurfaced` record for `turn_id` carries `memory` in its hits — the
/// structural evidence that the ambient pass put the target memory in front of the agent.
fn ambient_hits_include(events: &[Event], turn_id: TurnId, memory: MemoryId) -> bool {
    events.iter().any(|event| {
        matches!(
            &event.payload,
            EventPayload::AmbientRecallSurfaced { turn_id: hit_turn, hits, .. }
                if *hit_turn == turn_id && hits.iter().any(|hit| hit.memory == memory)
        )
    })
}
