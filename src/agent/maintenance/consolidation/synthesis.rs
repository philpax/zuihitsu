//! Cluster synthesis: the model call that merges a same-level cluster of entries into one richer
//! entry. Only tier 1 (within-level synthesis) calls this — tier 2 dedup writes no new text, so the
//! more-private text it retires never reaches a synthesis prompt.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    InstanceError,
    agent::{TurnError, turn::Recording},
    engine::Engine,
    event::{ModelPhase, Visibility},
    graph::{ClassLinkView, EntryView},
    ids::MemoryId,
    model::{GenerateRequest, ModelClient},
};

/// The model's synthesis response: the single consolidated text merging the cluster's entries.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct ConsolidationSynthesis {
    consolidated_text: String,
}

/// Call the model to synthesize a consolidated entry from a same-level `cluster`. Returns the merged
/// text, or `None` when the model declines or its response does not parse. The cluster's entries all
/// share one visibility level (and, below the public level, one teller), so the synthesized text stays
/// within a single audience — the more-private text of a cross-level near-duplicate is never mixed in.
pub(super) async fn synthesize_cluster(
    engine: &Engine,
    model: &dyn ModelClient,
    recording: &Recording,
    template_body: &str,
    memory_id: MemoryId,
    cluster: &[EntryView],
    existing_links: &[ClassLinkView],
) -> Result<Option<String>, InstanceError> {
    let entry_lines: Vec<String> = cluster
        .iter()
        .map(|entry| {
            format!(
                "- id: {}, text: {:?}, told_by: {:?}, visibility: {}",
                entry.entry_id.0,
                entry.text,
                entry.told_by,
                visibility_label(&entry.visibility)
            )
        })
        .collect();
    let link_lines: Vec<String> = {
        let graph = engine.graph.lock();
        existing_links
            .iter()
            .map(|link| {
                let to_name = graph
                    .memory_by_id(link.to)
                    .ok()
                    .flatten()
                    .map(|memory| memory.name.as_str().to_owned())
                    .unwrap_or_else(|| link.to.0.to_string());
                format!("- {} → {}", link.relation.as_str(), to_name)
            })
            .collect()
    };
    let user_prompt = format!(
        "Memory: {}\n\nEntries in this cluster:\n{}\n\nExisting links on this identity:\n{}",
        memory_id.0,
        entry_lines.join("\n"),
        if link_lines.is_empty() {
            "(none)".to_owned()
        } else {
            link_lines.join("\n")
        }
    );

    let request = GenerateRequest::structured::<ConsolidationSynthesis>(
        template_body,
        user_prompt,
        "consolidation_synthesis",
    );

    let record = recording.request_record(&request, None, &[]);
    let response = recording
        .generate(engine, model, &request, ModelPhase::Synthesis, record, None)
        .await
        .map_err(|error| InstanceError::from(TurnError::Model(error)))?
        .expect_completed();

    let Some(parsed) =
        crate::model::parse_structured::<ConsolidationSynthesis>(&response.completion)
    else {
        return Ok(None);
    };

    Ok(Some(parsed.consolidated_text))
}

/// The agent-facing label for an entry's visibility posture, shown in the synthesis prompt so the
/// model sees how widely each source surfaces.
fn visibility_label(visibility: &Visibility) -> &'static str {
    match visibility {
        Visibility::Public => "public",
        Visibility::Attributed => "attributed",
        Visibility::PrivateToTeller => "private",
        Visibility::Exclude(_) => "excluded",
    }
}
