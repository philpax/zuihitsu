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

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    engine::Engine,
    event::{Cardinality, EventPayload, LinkSource, ModelPhase, PromptTemplateName, Visibility},
    graph::{EntryView, MemoryView, RelationView},
    ids::{MemoryId, MemoryName, Seq, TurnId},
    model::{
        Completion, GenerateRequest, GenerateResponse, ModelClient, ModelError, extract_json_object,
    },
    settings::CaptureLevel,
    time::{self, Timestamp},
    vocabulary::RelationName,
};

use super::{Recording, TurnError, collect_written_memories, templates};

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
        // Read the memory, its same_as class's Public entries, its existing links, the registered
        // relations, and the candidate targets — all under a transient lock released before the
        // `.await`. No graph guard is held across a suspension point.
        let context = {
            let graph = engine.graph.lock();
            let Some(memory) = graph.memory_by_id(id)? else {
                continue;
            };
            // Class-wide read, same as describe: a merged identity's links are considered together.
            let entries = graph.class_entries(id)?;
            let public_entries: Vec<EntryView> = entries
                .iter()
                .filter(|entry| entry.visibility == Visibility::Public)
                .cloned()
                .collect();
            if public_entries.is_empty() {
                continue;
            }
            // Resolve each existing link's endpoints to handles so the prompt shows handles, not
            // raw ids — the model names handles, and a bare ULID is meaningless to it.
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
            // Candidates span all namespaces: a `topic/*` memory's "authored by Clara" entry points
            // to a `person/*` memory, so limiting to namespace siblings would exclude the target.
            let candidates = graph.memories_in_namespace("")?;
            InferenceContext {
                memory,
                entries: public_entries,
                existing_links,
                relations,
                candidates,
            }
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
            Ok(None) => continue,
            Err(error) => {
                tracing::warn!(
                    memory = %context.memory.name.as_str(),
                    %error,
                    "link inference failed; leaving the memory's links unchanged"
                );
                continue;
            }
        };

        // The set of relation labels the model can validly use on a link: those already registered,
        // plus those it just registered. A link whose relation is neither is dropped rather than
        // auto-registered with guessed defaults — lenient degradation that avoids committing a spec
        // the model did not endorse.
        let mut usable_relations: Vec<String> = context
            .relations
            .iter()
            .map(|r| r.name.as_str().to_owned())
            .collect();
        for spec in &result.new_relations {
            let label = spec.name.to_ascii_lowercase();
            if !usable_relations.contains(&label) {
                usable_relations.push(label.clone());
            }
            if label == RelationName::SameAs.as_str() {
                // The `same_as` guard: an inferred `same_as` would silently merge identities through
                // `recompute_classes`, bypassing the adjudication gate. The prompt forbids it too.
                continue;
            }
            if context.relations.iter().any(|r| r.name.as_str() == label) {
                // Already registered; a re-registration is a no-op upsert, so skip the event.
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
                name: RelationName::new(label.clone()),
                inverse: RelationName::new(spec.inverse.to_ascii_lowercase()),
                from_card,
                to_card,
                symmetric: spec.symmetric,
                reflexive: spec.reflexive,
            });
        }

        let graph = engine.graph.lock();
        for link in &result.links {
            // Lowercase before resolving so non-canonical casing like "Same_As" is caught.
            let relation_label = link.relation.to_ascii_lowercase();
            if relation_label == RelationName::SameAs.as_str() {
                continue;
            }
            if !usable_relations.contains(&relation_label) {
                tracing::debug!(
                    memory = %context.memory.name.as_str(),
                    relation = %link.relation,
                    "dropping a link whose relation is neither registered nor proposed"
                );
                continue;
            }
            // Resolve the target handle to a live memory — the model never invents ids. A handle it
            // cannot resolve is skipped, not minted.
            let Some(target_memory) = graph.memory_by_name(MemoryName::new(&link.target))? else {
                tracing::debug!(
                    memory = %context.memory.name.as_str(),
                    target = %link.target,
                    "dropping a link whose target does not resolve to a live memory"
                );
                continue;
            };
            let (from, to) = match link.direction.as_str() {
                "to" => (id, target_memory.id),
                "from" => (target_memory.id, id),
                other => {
                    tracing::debug!(
                        memory = %context.memory.name.as_str(),
                        direction = %other,
                        "dropping a link with an unrecognized direction"
                    );
                    continue;
                }
            };
            // Skip a duplicate of an existing edge — the model was shown them, and the graph's
            // `INSERT OR IGNORE` makes a duplicate a no-op anyway, so skip the event.
            if context.existing_links.iter().any(|existing| {
                existing.from_id == from
                    && existing.to_id == to
                    && existing.relation == RelationName::new(&relation_label)
            }) {
                continue;
            }
            events.push(EventPayload::LinkCreated {
                from,
                to,
                relation: RelationName::new(&relation_label),
                source: LinkSource::Inferred,
                // No teller behind an inferred link, same as the adjudicated `same_as`.
                told_by: None,
            });
        }
    }

    if !events.is_empty() {
        engine.store.lock().append(now, events)?;
        // Graph (written) before store (read), per the lock-ordering rule.
        let mut graph = engine.graph.lock();
        graph.materialize_from(engine.store.lock().as_ref())?;
    }
    Ok(())
}

/// The gathered inputs for one memory's inference, held outside the graph lock.
struct InferenceContext {
    memory: MemoryView,
    entries: Vec<EntryView>,
    /// Existing links with both their raw endpoint ids (for dedup) and resolved handles (for the
    /// prompt, where a bare ULID is meaningless to the model).
    existing_links: Vec<ExistingLink>,
    relations: Vec<RelationView>,
    candidates: Vec<MemoryView>,
}

/// An existing link, carrying both the raw endpoint ids and the resolved handles.
struct ExistingLink {
    from_id: MemoryId,
    to_id: MemoryId,
    from: MemoryName,
    to: MemoryName,
    relation: RelationName,
}

/// Ask the model, in one schema-constrained reply, to identify relationships in the memory's
/// statements that link it to one of the candidate memories. The statements are numbered (1-based)
/// so the model can key a link back to the entry that grounds it; the existing links and registered
/// relations are listed so it can avoid duplicates and reuse labels. `None` means no usable reply,
/// which the caller treats as "leave the memory unchanged".
async fn infer_relationships(
    model: &dyn ModelClient,
    engine: &Engine,
    recording: Recording,
    system: &str,
    context: &InferenceContext,
    now: Timestamp,
) -> Result<Option<LinkInferenceArgs>, ModelError> {
    let prompt = render_prompt(
        &context.memory,
        &context.entries,
        &context.existing_links,
        &context.relations,
        &context.candidates,
        now,
    );
    let request =
        GenerateRequest::structured::<LinkInferenceArgs>(system, prompt, "link_inference");
    const ATTEMPTS: usize = 3;
    for attempt in 1..=ATTEMPTS {
        let record = recording.request_record(&request, None);
        let GenerateResponse { completion, .. } = recording
            .generate(engine, model, &request, ModelPhase::Synthesis, record)
            .await?;
        if let Completion::Reply(content) = completion
            && let Some(json) = extract_json_object(&content)
            && let Ok(value) = serde_json::from_str::<serde_json::Value>(json)
            && let Some(args) = link_inference_argument(&value)
        {
            return Ok(Some(args));
        }
        tracing::debug!(
            memory = %context.memory.name.as_str(),
            attempt,
            "link inference returned no usable JSON"
        );
    }
    tracing::warn!(
        memory = %context.memory.name.as_str(),
        attempts = ATTEMPTS,
        "link inference gave up after retries; keeping the memory's links unchanged"
    );
    Ok(None)
}

/// Render the inference prompt: the memory and its numbered statements, its existing links, the
/// registered relations, and the candidate target memories by handle and description.
fn render_prompt(
    memory: &MemoryView,
    entries: &[EntryView],
    existing_links: &[ExistingLink],
    relations: &[RelationView],
    candidates: &[MemoryView],
    now: Timestamp,
) -> String {
    let mut prompt = format!(
        "Memory: {}\nCurrent time: {}\n\nStatements:\n",
        memory.name.as_str(),
        time::format_datetime(now),
    );
    for (index, entry) in entries.iter().enumerate() {
        prompt.push_str(&format!("{}. {}\n", index + 1, entry.text));
    }
    prompt.push_str("\nExisting links:\n");
    if existing_links.is_empty() {
        prompt.push_str("  (none)\n");
    } else {
        for link in existing_links {
            prompt.push_str(&format!(
                "- {} —{}→ {}\n",
                link.from.as_str(),
                link.relation.as_str(),
                link.to.as_str()
            ));
        }
    }
    prompt.push_str("\nRegistered relations:\n");
    if relations.is_empty() {
        prompt.push_str("  (none)\n");
    } else {
        for relation in relations {
            prompt.push_str(&format!(
                "- {}/{} (from: {}, to: {}, symmetric: {}, reflexive: {})\n",
                relation.name.as_str(),
                relation.inverse.as_str(),
                relation.from_card.as_str(),
                relation.to_card.as_str(),
                relation.symmetric,
                relation.reflexive,
            ));
        }
    }
    prompt.push_str("\nCandidate memories (resolve relationships to these handles):\n");
    for candidate in candidates.iter().take(CANDIDATE_CAP) {
        prompt.push_str(&format!(
            "- {} — {}\n",
            candidate.name.as_str(),
            candidate.description
        ));
    }
    prompt.push_str("\nIdentify relationships in the statements that link this memory to one of the candidates.\n");
    prompt
}

/// The `link_inference` reply shape; doubles as the schema sent to the model, so prompt and parser
/// cannot drift. Constructed directly by tests so the JSON a test emits cannot drift from the schema.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct LinkInferenceArgs {
    /// New relation types to register before creating links of that type. May be empty.
    #[serde(default)]
    pub new_relations: Vec<NewRelationSpec>,
    /// Relationships to create. May be empty.
    #[serde(default)]
    pub links: Vec<InferredLink>,
}

/// A relation the model coins for a relationship no registered relation fits.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct NewRelationSpec {
    pub name: String,
    pub inverse: String,
    pub from_card: String,
    pub to_card: String,
    pub symmetric: bool,
    pub reflexive: bool,
}

/// A relationship the model identifies, grounded in a numbered statement.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct InferredLink {
    /// The statement number (1-based) that grounds this relationship. Read by the model to cite its
    /// basis; not consumed by the pass (the relation and target are sufficient to create the link).
    #[allow(dead_code)]
    pub entry: usize,
    /// The relation label. Must be a registered relation or one in `new_relations`.
    pub relation: String,
    /// The candidate memory's handle, e.g. "person/clara".
    pub target: String,
    /// "to" (subject → target) or "from" (target → subject).
    pub direction: String,
}

/// Parse a structured reply leniently. A well-formed `links` array with a malformed `new_relations`
/// entry still produces the links that do not need a new relation, rather than discarding the whole
/// reply on one bad field — the same salvage discipline as `synthesize_argument`. The caller is
/// responsible for extracting the JSON object from the model's fenced reply; this function takes the
/// parsed `Value` and salvages each field independently.
fn link_inference_argument(value: &serde_json::Value) -> Option<LinkInferenceArgs> {
    let new_relations = value
        .get("new_relations")
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| serde_json::from_value::<NewRelationSpec>(item.clone()).ok())
                .collect()
        })
        .unwrap_or_default();
    let links = value
        .get("links")
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| serde_json::from_value::<InferredLink>(item.clone()).ok())
                .collect()
        })
        .unwrap_or_default();
    Some(LinkInferenceArgs {
        new_relations,
        links,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_well_formed_reply_parses_into_relations_and_links() {
        let reply = serde_json::json!({
            "new_relations": [{
                "name": "authored_by",
                "inverse": "authored",
                "from_card": "many",
                "to_card": "one",
                "symmetric": false,
                "reflexive": false
            }],
            "links": [{
                "entry": 1,
                "relation": "authored_by",
                "target": "person/clara",
                "direction": "to"
            }]
        });
        let args = link_inference_argument(&reply).expect("a well-formed reply parses");
        assert_eq!(args.new_relations.len(), 1);
        assert_eq!(args.new_relations[0].name, "authored_by");
        assert_eq!(args.new_relations[0].inverse, "authored");
        assert_eq!(args.links.len(), 1);
        assert_eq!(args.links[0].target, "person/clara");
        assert_eq!(args.links[0].direction, "to");
    }

    #[test]
    fn a_malformed_new_relation_is_skipped_while_links_survive() {
        // A `new_relations` entry missing `inverse` fails the per-item parse and is dropped, but the
        // `links` array — well-formed on its own — is salvaged.
        let reply = serde_json::json!({
            "new_relations": [{ "name": "authored_by" }],
            "links": [{
                "entry": 1,
                "relation": "knows",
                "target": "person/clara",
                "direction": "to"
            }]
        });
        let args = link_inference_argument(&reply).expect("the links are salvaged");
        assert!(args.new_relations.is_empty());
        assert_eq!(args.links.len(), 1);
        assert_eq!(args.links[0].relation, "knows");
    }

    #[test]
    fn a_malformed_link_is_skipped_while_relations_survive() {
        let reply = serde_json::json!({
            "new_relations": [{
                "name": "authored_by",
                "inverse": "authored",
                "from_card": "many",
                "to_card": "one",
                "symmetric": false,
                "reflexive": false
            }],
            "links": [{ "entry": 1, "relation": "authored_by" }]
        });
        let args = link_inference_argument(&reply).expect("the relations are salvaged");
        assert_eq!(args.new_relations.len(), 1);
        assert!(args.links.is_empty());
    }

    #[test]
    fn a_reply_with_no_links_or_relations_parses_to_empty() {
        let reply = serde_json::json!({ "new_relations": [], "links": [] });
        let args = link_inference_argument(&reply).expect("an empty reply parses");
        assert!(args.new_relations.is_empty());
        assert!(args.links.is_empty());
    }
}
