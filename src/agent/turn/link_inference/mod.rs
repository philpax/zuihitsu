//! The off-hot-path link inference: for each memory whose content changed, identify relationships
//! implicit in the content and assert them as links (spec §Write path → link inference).
//!
//! A sibling to the describer and adjudicator passes — a separate catch-up rather than an extra duty
//! bolted onto the description-regen prompt, so one focused model call extracts relationships while
//! the description call stays on its own concern. It reads each written memory's Public entries
//! (private asides never become graph edges here), its existing links (to avoid duplicates), and the
//! registered relations (to reuse before coining), then asks the model to identify relationships that
//! link the memory to one of the candidate target memories. New relations are registered before the
//! links of that type are created. The model never invents ids: it names handles, and the pass
//! resolves each via `memory_by_name`, skipping one it cannot resolve (an unresolved entity is not a
//! link to create, never a new memory). The pass never infers `same_as` — those flow through the
//! adjudication gate, not here — and carries no teller behind its links, like the adjudicated
//! `same_as`. Idempotent: a duplicate `LinkCreated` is a graph no-op (`INSERT OR IGNORE`), a
//! `LinkTypeRegistered` is an upsert, and the cursor advance keeps a window from being re-scanned.

mod argument;
mod prompt;
mod relations;
#[cfg(test)]
mod tests;

use crate::{
    engine::Engine,
    event::{
        Cardinality, EventPayload, EventSource, InferredLinkSpec, InferredRelationSpec,
        LinkInferenceResult, LinkSource, ProducedBy, PromptTemplateName, Visibility,
    },
    graph::EntryView,
    ids::{MemoryId, MemoryName, Seq, TurnId},
    model::ModelClient,
    settings::CaptureLevel,
    vocabulary::RelationName,
};

use super::{Recording, TurnError, collect_written_memories, templates};

use argument::link_inference_argument;
pub use argument::{InferredLink, LinkInferenceArgs, NewRelationSpec};
use relations::{ExistingLink, InferenceContext, infer_relationships};

/// The cap on how many candidate target memories are listed in a single inference prompt. A
/// prompt-size bound, not a correctness constraint: a handle the model names that is beyond the cap
/// is still resolved by `memory_by_name`, so a memory left out of the candidates is not lost.
const CANDIDATE_CAP: usize = 100;

/// Catch link inference up to the log (spec §Write path → link inference): for each memory whose
/// content changed in `(cursor, head]`, identify the relationships implicit in its Public entries and
/// commit any new relation registrations and links. Returns the head it advanced to and how many
/// memories it considered. Gated by the `LinkInference` template existing — no template, no-op — so
/// the feature is toggled by whether its prompt is registered. The cursor always advances to `head`
/// regardless of the toggle, matching the describer and adjudicator precedents, so a toggled-off
/// pass does not re-scan the window. Idempotent: re-running from the same cursor produces no new
/// events.
pub async fn run_link_inference_catch_up(
    engine: &Engine,
    model: &dyn ModelClient,
    cursor: Seq,
) -> Result<(Seq, usize), TurnError> {
    let head = engine.store.lock().head()?;
    if head <= cursor {
        return Ok((cursor, 0));
    }
    let written = collect_written_memories(engine.store.lock().as_ref(), cursor)?;
    infer_links(
        model,
        engine,
        &written,
        Recording {
            conversation: None,
            turn_id: TurnId::generate(),
            capture: CaptureLevel::Off,
        },
    )
    .await?;
    Ok((head, written.len()))
}

/// Infer relationships for each written memory: read its Public entries, existing links, and the
/// registered relations, ask the model to identify relationships in one schema-constrained call, and
/// commit any new registrations and links in one batch. A memory with no Public entries is skipped;
/// a model failure on one memory is logged and leaves it unchanged rather than failing the rest. The
/// feature toggle lives here, inside the inner function, matching the adjudicate and describe
/// precedents: no `LinkInference` template registered, the pass emits no events.
async fn infer_links(
    model: &dyn ModelClient,
    engine: &Engine,
    written: &[MemoryId],
    recording: Recording,
) -> Result<(), TurnError> {
    let Some(template) = templates::latest_template(
        engine.store.lock().as_ref(),
        PromptTemplateName::LinkInference,
    )?
    else {
        return Ok(());
    };
    let now = engine.clock.now();
    let mut events = Vec::new();
    for &id in written {
        let context = {
            let graph = engine.graph.lock();
            let Some(memory) = graph.memory_by_id(id)? else {
                continue;
            };
            let entries = graph.class_entries(id)?;
            let public_entries: Vec<EntryView> = entries
                .iter()
                .filter(|entry| entry.visibility == Visibility::Public)
                .cloned()
                .collect();
            if public_entries.is_empty() {
                continue;
            }
            let raw_links = graph.links(id)?;
            let mut existing_links = Vec::with_capacity(raw_links.len());
            for link in raw_links {
                let from_name = graph
                    .memory_by_id(link.from)?
                    .map(|m| m.name)
                    .unwrap_or_else(|| MemoryName::new("<deleted>"));
                let to_name = graph
                    .memory_by_id(link.to)?
                    .map(|m| m.name)
                    .unwrap_or_else(|| MemoryName::new("<deleted>"));
                existing_links.push(ExistingLink {
                    from_id: link.from,
                    to_id: link.to,
                    from: from_name,
                    to: to_name,
                    relation: link.relation,
                });
            }
            let relations = graph.all_relations()?;
            let candidates = graph.memories_in_namespace("")?;
            InferenceContext {
                memory,
                entries: public_entries,
                existing_links,
                relations,
                candidates,
            }
        };

        let produced_by = ProducedBy {
            model_id: model.model_id().into(),
            template_name: PromptTemplateName::LinkInference,
            template_version: template.version,
        };

        let result = match infer_relationships(
            model,
            engine,
            recording,
            &template.body,
            &context,
            now,
        )
        .await
        {
            Ok(Some(result)) => result,
            Ok(None) => {
                events.push(EventPayload::LinksInferred {
                    memory: id,
                    result: LinkInferenceResult::default(),
                    produced_by: Some(produced_by),
                });
                continue;
            }
            Err(error) => {
                tracing::warn!(
                    memory = %context.memory.name.as_str(),
                    %error,
                    "link inference failed; leaving the memory's links unchanged"
                );
                continue;
            }
        };

        events.push(EventPayload::LinksInferred {
            memory: id,
            result: LinkInferenceResult {
                new_relations: result
                    .new_relations
                    .iter()
                    .map(|spec| InferredRelationSpec {
                        name: spec.name.clone(),
                        inverse: spec.inverse.clone(),
                        from_card: spec.from_card.clone(),
                        to_card: spec.to_card.clone(),
                        symmetric: spec.symmetric,
                        reflexive: spec.reflexive,
                        description: spec.description.clone(),
                    })
                    .collect(),
                links: result
                    .links
                    .iter()
                    .map(|link| {
                        // The record keeps its target/direction wire shape; both derive from the
                        // sentence. A sentence that does not name this memory as its subject reads
                        // as "from" (target → subject), matching the consumption below.
                        let subject_is_memory = link.subject == context.memory.name.as_str();
                        InferredLinkSpec {
                            entry: link.entry,
                            relation: link.relation.clone(),
                            target: if subject_is_memory {
                                link.object.clone()
                            } else {
                                link.subject.clone()
                            },
                            direction: if subject_is_memory { "to" } else { "from" }.to_owned(),
                        }
                    })
                    .collect(),
            },
            produced_by: Some(produced_by),
        });

        let mut usable_relations: Vec<(String, String)> = context
            .relations
            .iter()
            .map(|r| (r.name.as_str().to_owned(), r.inverse.as_str().to_owned()))
            .collect();
        for spec in &result.new_relations {
            let label = spec.name.to_ascii_lowercase();
            if !usable_relations.iter().any(|(name, _)| *name == label) {
                usable_relations.push((label.clone(), spec.inverse.to_ascii_lowercase()));
            }
            if label == RelationName::SameAs.as_str() {
                continue;
            }
            if context.relations.iter().any(|r| r.name.as_str() == label) {
                continue;
            }
            let from_card = match spec.from_card.parse::<Cardinality>() {
                Ok(card) => card,
                Err(_) => {
                    tracing::debug!(
                        memory = %context.memory.name.as_str(),
                        relation = %spec.name,
                        from_card = %spec.from_card,
                        "dropping a new relation with an unparseable from_card"
                    );
                    continue;
                }
            };
            let to_card = match spec.to_card.parse::<Cardinality>() {
                Ok(card) => card,
                Err(_) => {
                    tracing::debug!(
                        memory = %context.memory.name.as_str(),
                        relation = %spec.name,
                        to_card = %spec.to_card,
                        "dropping a new relation with an unparseable to_card"
                    );
                    continue;
                }
            };
            events.push(EventPayload::LinkTypeRegistered {
                name: RelationName::new(&label),
                inverse: RelationName::new(&spec.inverse.to_ascii_lowercase()),
                from_card,
                to_card,
                symmetric: spec.symmetric,
                reflexive: spec.reflexive,
                description: spec.description.clone(),
            });
        }

        let graph = engine.graph.lock();
        for link in &result.links {
            let relation_label = link.relation.to_ascii_lowercase();
            if relation_label == RelationName::SameAs.as_str() {
                continue;
            }
            let Some((canonical_label, via_inverse)) =
                usable_relations.iter().find_map(|(name, inverse)| {
                    if relation_label == *name {
                        Some((name.clone(), false))
                    } else if relation_label == *inverse {
                        Some((name.clone(), true))
                    } else {
                        None
                    }
                })
            else {
                tracing::debug!(
                    memory = %context.memory.name.as_str(),
                    relation = %link.relation,
                    "dropping a link whose relation is neither registered nor proposed"
                );
                continue;
            };
            // The sentence "subject relation object" carries the direction: exactly one endpoint
            // must be the memory under inference, and the other names the linked memory. A sentence
            // that names this memory on neither or both ends is unusable.
            let memory_name = context.memory.name.as_str();
            let target_label = match (link.subject == memory_name, link.object == memory_name) {
                (true, false) => &link.object,
                (false, true) => &link.subject,
                _ => {
                    tracing::debug!(
                        memory = %memory_name,
                        subject = %link.subject,
                        object = %link.object,
                        "dropping a link whose sentence does not have this memory on exactly one end"
                    );
                    continue;
                }
            };
            let Some(target_memory) = graph.memory_by_name(MemoryName::new(target_label))? else {
                tracing::debug!(
                    memory = %memory_name,
                    target = %target_label,
                    "dropping a link whose target does not resolve to a live memory"
                );
                continue;
            };
            let (from, to) = if link.subject == memory_name {
                (id, target_memory.id)
            } else {
                (target_memory.id, id)
            };
            let (from, to) = if via_inverse { (to, from) } else { (from, to) };
            if context.existing_links.iter().any(|existing| {
                existing.from_id == from
                    && existing.to_id == to
                    && existing.relation == RelationName::new(&canonical_label)
            }) {
                continue;
            }
            // Inherit the visibility of the source entry the link was extracted from, so a link
            // inferred from a private entry is itself private (spec §Visibility → inferred links).
            // The 1-based entry index into context.entries.
            let source_visibility = link
                .entry
                .checked_sub(1)
                .and_then(|idx| context.entries.get(idx))
                .map(|entry| entry.visibility.clone())
                .unwrap_or(Visibility::Public);
            events.push(EventPayload::link_created(
                from,
                to,
                RelationName::new(&canonical_label),
                LinkSource::Inferred,
                None,
                None,
                source_visibility,
            ));
        }
    }

    if !events.is_empty() {
        engine
            .store
            .lock()
            .append(now, EventSource::Orchestration, events)?;
        let mut graph = engine.graph.lock();
        graph.materialize_from(engine.store.lock().as_ref())?;
    }
    Ok(())
}
