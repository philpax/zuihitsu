//! The inference context and the model call that identifies relationships.

use crate::{
    engine::Engine,
    event::ModelPhase,
    graph::{EntryView, MemoryView, RelationView},
    ids::{MemoryId, MemoryName},
    model::{
        Completion, GenerateRequest, GenerateResponse, ModelClient, ModelError, extract_json_object,
    },
    time::Timestamp,
    vocabulary::RelationName,
};

use super::{LinkInferenceArgs, Recording, link_inference_argument, prompt::render_prompt};

/// The gathered inputs for one memory's inference, held outside the graph lock.
pub(super) struct InferenceContext {
    pub(super) memory: MemoryView,
    pub(super) entries: Vec<EntryView>,
    /// Existing links with both their raw endpoint ids (for dedup) and resolved handles (for the
    /// prompt, where a bare ULID is meaningless to the model).
    pub(super) existing_links: Vec<ExistingLink>,
    pub(super) relations: Vec<RelationView>,
    pub(super) candidates: Vec<MemoryView>,
}

/// An existing link, carrying both the raw endpoint ids and the resolved handles.
pub(super) struct ExistingLink {
    pub(super) from_id: MemoryId,
    pub(super) to_id: MemoryId,
    pub(super) from: MemoryName,
    pub(super) to: MemoryName,
    pub(super) relation: RelationName,
}

/// Ask the model, in one schema-constrained reply, to identify relationships in the memory's
/// statements that link it to one of the candidate memories. The statements are numbered (1-based)
/// so the model can key a link back to the entry that grounds it; the existing links and registered
/// relations are listed so it can avoid duplicates and reuse labels. `None` means no usable reply,
/// which the caller treats as "leave the memory unchanged".
pub(super) async fn infer_relationships(
    model: &dyn ModelClient,
    engine: &Engine,
    recording: &Recording,
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
        // The link-inference prompt is not the six-section assembled prompt, so it carries no typed
        // section spans.
        let record = recording.request_record(&request, None, &[]);
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
