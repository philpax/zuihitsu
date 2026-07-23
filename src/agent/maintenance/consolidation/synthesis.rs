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
    ids::{EntryId, MemoryId},
    model::{GenerateRequest, ModelClient},
};

/// The model's synthesis response: which of the candidate entries state the same fact and belong
/// together, and the single consolidated text merging exactly those. Geometry gathers the candidate
/// cluster loosely (at `consolidation_candidate_threshold`); this response is where the model disposes,
/// selecting the true membership and leaving a related-but-distinct candidate out.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct ConsolidationSynthesis {
    /// The ids of the candidate entries that state the same fact (or aspects of one fact) and are
    /// merged into `consolidated_text`. Fewer than two selections is a decline.
    selected_entry_ids: Vec<String>,
    consolidated_text: String,
}

/// The model's membership decision for a candidate cluster: the consolidated text and the subset of
/// cluster entries it merges. Only these selected sources are consolidated; the unselected candidates
/// stay live.
pub(super) struct Synthesis {
    pub text: String,
    pub selected: Vec<EntryId>,
}

/// Call the model to select and synthesize a consolidated entry from a same-level candidate `cluster`.
/// Returns the merged text paired with the selected source subset, or `None` when the model declines,
/// its response does not parse, or fewer than two candidates are validly selected. The cluster's
/// entries all share one visibility level (and, below the public level, one teller), so the synthesized
/// text stays within a single audience — the more-private text of a cross-level near-duplicate is never
/// mixed in. The candidate cluster is gathered at the loose `consolidation_candidate_threshold`, so it
/// may hold a related-but-distinct entry geometry could not separate; the model's `selected_entry_ids`
/// is what narrows it to the entries that state one fact.
pub(super) async fn synthesize_cluster(
    engine: &Engine,
    model: &dyn ModelClient,
    recording: &Recording,
    template_body: &str,
    memory_id: MemoryId,
    cluster: &[EntryView],
    existing_links: &[ClassLinkView],
) -> Result<Option<Synthesis>, InstanceError> {
    let entry_lines: Vec<String> = cluster
        .iter()
        .enumerate()
        .map(|(n, entry)| {
            format!(
                "{}. id: {}, text: {:?}, told_by: {:?}, visibility: {}",
                n + 1,
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

    let selected = validate_selection(cluster, &parsed.selected_entry_ids, memory_id);
    // A selection of fewer than two valid members is a decline: there is nothing to consolidate, and
    // every candidate stays live.
    if selected.len() < 2 {
        tracing::debug!(
            memory = ?memory_id,
            valid_selections = selected.len(),
            "consolidation: fewer than two candidates validly selected; declining the cluster"
        );
        return Ok(None);
    }

    Ok(Some(Synthesis {
        text: parsed.consolidated_text,
        selected,
    }))
}

/// Resolve the model's selected ids against the candidate `cluster`'s members, in the model's order.
/// An id that resolves to no cluster member is a model slip — log-warn and drop it rather than fail the
/// sweep; a repeated id collapses to one. The result is the validated source subset the caller
/// consolidates (or, when it holds fewer than two members, declines on).
fn validate_selection(
    cluster: &[EntryView],
    raw_ids: &[String],
    memory_id: MemoryId,
) -> Vec<EntryId> {
    let mut selected: Vec<EntryId> = Vec::new();
    for raw in raw_ids {
        match cluster
            .iter()
            .find(|entry| entry.entry_id.0.to_string() == *raw)
        {
            Some(entry) if !selected.contains(&entry.entry_id) => selected.push(entry.entry_id),
            Some(_) => {}
            None => tracing::warn!(
                memory = ?memory_id,
                selected_id = %raw,
                "consolidation: model selected an id absent from the candidate cluster; dropping it"
            ),
        }
    }
    selected
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

#[cfg(test)]
mod tests {
    use super::validate_selection;
    use crate::{
        event::{Teller, Visibility},
        graph::{EntryOrigin, EntryView},
        ids::{EntryId, MemoryId},
        time::Timestamp,
    };

    /// A minimal candidate-cluster member carrying just the id the selection logic keys on.
    fn member() -> EntryView {
        EntryView {
            entry_id: EntryId::generate(),
            asserted_at: Timestamp::from_millis(1_000),
            occurred_sort: None,
            occurred_at: None,
            occurred_authored: false,
            text: "a fact".to_owned(),
            told_by: Teller::Agent,
            told_in: None,
            visibility: Visibility::Public,
            superseded_by: None,
            retracted_reason: None,
            origin: EntryOrigin::Recorded,
            attestations: Vec::new(),
        }
    }

    fn id_string(entry: &EntryView) -> String {
        entry.entry_id.0.to_string()
    }

    #[test]
    fn selects_only_the_named_subset() {
        // Three candidates gathered by the loose bar; the model names two of them, so only those two
        // are the consolidation sources and the third stays live.
        let cluster = vec![member(), member(), member()];
        let raw = vec![id_string(&cluster[0]), id_string(&cluster[2])];
        let selected = validate_selection(&cluster, &raw, MemoryId::generate());
        assert_eq!(
            selected,
            vec![cluster[0].entry_id, cluster[2].entry_id],
            "the two named members are selected in model order; the unnamed third is left out"
        );
    }

    #[test]
    fn drops_an_id_absent_from_the_cluster() {
        // An id the model invented (or one for an entry outside this cluster) resolves to no member,
        // so it is dropped and the valid selections proceed.
        let cluster = vec![member(), member()];
        let raw = vec![
            id_string(&cluster[0]),
            "not-a-cluster-member".to_owned(),
            id_string(&cluster[1]),
        ];
        let selected = validate_selection(&cluster, &raw, MemoryId::generate());
        assert_eq!(
            selected,
            vec![cluster[0].entry_id, cluster[1].entry_id],
            "the unknown id is dropped and the two valid members remain"
        );
    }

    #[test]
    fn collapses_a_repeated_id() {
        let cluster = vec![member(), member()];
        let raw = vec![
            id_string(&cluster[0]),
            id_string(&cluster[0]),
            id_string(&cluster[1]),
        ];
        let selected = validate_selection(&cluster, &raw, MemoryId::generate());
        assert_eq!(
            selected,
            vec![cluster[0].entry_id, cluster[1].entry_id],
            "a repeated id collapses to a single selection"
        );
    }

    #[test]
    fn a_single_valid_selection_is_below_the_merge_floor() {
        // One valid id plus one unknown leaves a single validated member — the caller declines, since a
        // consolidation needs at least two sources.
        let cluster = vec![member(), member()];
        let raw = vec![id_string(&cluster[0]), "bogus".to_owned()];
        let selected = validate_selection(&cluster, &raw, MemoryId::generate());
        assert_eq!(
            selected,
            vec![cluster[0].entry_id],
            "only the one valid member survives, so the caller will decline (< 2 sources)"
        );
    }
}
