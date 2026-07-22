//! The consolidation pass: clusters semantically-overlapping live entries and synthesizes
//! richer consolidated replacements.
//!
//! Per identity class with content changes since the cursor:
//! 1. Gather live class entries.
//! 2. Embed all entries, then cluster by cosine similarity using complete linkage at the
//!    `consolidation_similarity_threshold`.
//! 3. For each cluster (≥2 entries), call the model to synthesize a consolidated entry, absorbing
//!    any entries whose content is purely a description of an existing link.
//! 4. Commit: append the replacement entry (`MemoryContentAppended` with `Teller::Agent`), then
//!    emit `EntriesConsolidated` to tombstone the sources.

use std::collections::BTreeSet;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    IndexError, InstanceError,
    agent::{TurnError, templates},
    engine::Engine,
    event::{
        EventPayload, EventSource, ModelPhase, ProducedBy, PromptTemplateName, Teller, Visibility,
    },
    graph::EntryView,
    ids::{EntryId, MemoryId, Seq, TurnId},
    model::{GenerateRequest, ModelClient, index::VectorKey},
    settings::{CaptureLevel, Settings},
    vector::VectorRecord,
};

use crate::agent::turn::{Recording, collect_written_memories};

/// The maximum number of entries per class the pass considers for clustering. A safety valve,
/// not a tuning parameter — clustering is O(n²) but trivially fast for n ≤ 100.
const MAX_ENTRIES_PER_CLASS: usize = 100;

/// Run one consolidation sweep. Returns `(new_cursor, memories_considered)`.
pub async fn catch_up(
    engine: &Engine,
    model: &dyn ModelClient,
    cursor: Seq,
) -> Result<(Seq, usize), InstanceError> {
    let head = engine.store.lock().head()?;
    if head <= cursor {
        return Ok((cursor, 0));
    }

    let Some(template) = templates::latest_template(
        engine.store.lock().as_ref(),
        PromptTemplateName::EntryConsolidation,
    )?
    else {
        return Ok((head, 0));
    };

    let written = collect_written_memories(engine.store.lock().as_ref(), cursor)?;
    if written.is_empty() {
        return Ok((head, 0));
    }

    let recording = Recording::new(None, TurnId::generate(), CaptureLevel::Off);
    let now = engine.clock.now();
    let settings = Settings::from_store(engine.store.lock().as_ref()).unwrap_or_default();
    let threshold = settings.maintenance.consolidation_similarity_threshold;
    let mut events = Vec::new();

    for &id in &written {
        let entries: Vec<EntryView> = {
            let graph = engine.graph.lock();
            graph.class_entries(id)?
        };
        if entries.len() < 2 || entries.len() > MAX_ENTRIES_PER_CLASS {
            continue;
        }

        let clusters = cluster_entries(engine, &entries, threshold).await?;

        for cluster in clusters {
            if cluster.len() < 2 {
                continue;
            }

            let existing_links = {
                let graph = engine.graph.lock();
                graph.class_links(id)?
            };

            let produced_by = ProducedBy {
                model_id: model.model_id().into(),
                template_name: PromptTemplateName::EntryConsolidation,
                template_version: template.version,
            };

            match synthesize_cluster(
                engine,
                model,
                &recording,
                &template.body,
                id,
                &cluster,
                &existing_links,
            )
            .await
            {
                Ok(Some(synthesis)) => {
                    let visibility = least_restrictive_visibility(&cluster);
                    let replacement = EntryId::generate();
                    events.push(EventPayload::MemoryContentAppended {
                        id,
                        entry_id: replacement,
                        asserted_at: now,
                        occurred_at: None,
                        text: synthesis.consolidated_text,
                        told_by: Teller::Agent,
                        told_in: None,
                        visibility,
                    });

                    let sources: Vec<EntryId> = cluster.iter().map(|e| e.entry_id).collect();
                    events.push(EventPayload::entries_consolidated(
                        id,
                        sources,
                        replacement,
                        Some(produced_by),
                    ));
                }
                Ok(None) => {
                    tracing::debug!(
                        memory = ?id,
                        cluster_size = cluster.len(),
                        "consolidation: model returned no synthesis for a cluster; skipping"
                    );
                }
                Err(error) => {
                    tracing::warn!(
                        memory = ?id,
                        %error,
                        "consolidation: synthesis failed for a cluster; skipping"
                    );
                }
            }
        }
    }

    if !events.is_empty() {
        engine
            .store
            .lock()
            .append(now, EventSource::Orchestration, events)?;
        engine
            .graph
            .lock()
            .materialize_from(engine.store.lock().as_ref())?;
    }

    Ok((head, written.len()))
}

/// The model's synthesis response: the consolidated text and the set of entry ids whose content
/// is fully captured by an existing link (absorbed).
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct ConsolidationSynthesis {
    consolidated_text: String,
    #[serde(default)]
    absorbed_entry_ids: Vec<String>,
}

/// Cluster a set of entries by cosine similarity using complete linkage at `threshold`.
/// Returns clusters of ≥1 entries; singletons are included so the caller can skip them.
async fn cluster_entries(
    engine: &Engine,
    entries: &[EntryView],
    threshold: f64,
) -> Result<Vec<Vec<EntryView>>, InstanceError> {
    let Some(retrieval) = &engine.retrieval else {
        return Ok(Vec::new());
    };

    // Embed all entries in one batch.
    let texts: Vec<String> = entries.iter().map(|e| e.text.clone()).collect();
    let embeddings = retrieval
        .embedder
        .embed(&texts)
        .await
        .map_err(|e| InstanceError::from(TurnError::Model(e)))?;

    // Insert them into the vector index so future passes and searches find them.
    {
        let mut vectors = retrieval.vectors.lock();
        for (entry, embedding) in entries.iter().zip(embeddings.iter()) {
            vectors
                .upsert(VectorRecord {
                    id: VectorKey::Entry(entry.entry_id).to_vector_id(),
                    embedding: embedding.clone(),
                    model_id: retrieval.embedder.model_id().into(),
                })
                .map_err(IndexError::Vector)?;
        }
    }

    if embeddings.len() < 2 {
        return Ok(Vec::new());
    }

    // Build condensed dissimilarity matrix (upper triangle, row-major).
    let n = embeddings.len();
    let mut dissimilarities: Vec<f32> = Vec::with_capacity(n * (n - 1) / 2);
    for i in 0..n {
        for j in (i + 1)..n {
            let sim = dot_product(&embeddings[i], &embeddings[j]);
            dissimilarities.push(1.0 - sim);
        }
    }

    // Run hierarchical clustering with complete linkage.
    let dendrogram = kodama::linkage(&mut dissimilarities, n, kodama::Method::Complete);

    // Cut the dendrogram at the threshold to get flat clusters.
    let labels = cut_tree(dendrogram.steps(), n, threshold as f32);

    // Group entries by cluster label.
    let mut clusters: Vec<Vec<EntryView>> = Vec::new();
    let mut label_to_idx: Vec<Option<usize>> = vec![None; n];
    for (i, &label) in labels.iter().enumerate() {
        if let Some(slot) = label_to_idx.get(label).and_then(|s| *s) {
            clusters[slot].push(entries[i].clone());
        } else {
            let slot = clusters.len();
            label_to_idx[label] = Some(slot);
            clusters.push(vec![entries[i].clone()]);
        }
    }

    Ok(clusters)
}

/// Cut a dendrogram at a given dissimilarity threshold, returning a flat cluster label per point.
fn cut_tree(steps: &[kodama::Step<f32>], n: usize, threshold: f32) -> Vec<usize> {
    // Each point starts as its own cluster.
    let mut labels: Vec<usize> = (0..n).collect();
    // Track which cluster each dendrogram node maps to.
    let mut node_cluster: Vec<usize> = (0..n).collect();

    for (next_cluster, step) in (n..).zip(steps.iter()) {
        if step.dissimilarity > threshold {
            break;
        }
        let a = node_cluster[step.cluster1];
        let b = node_cluster[step.cluster2];
        let merged = next_cluster;
        for label in labels.iter_mut() {
            if *label == a || *label == b {
                *label = merged;
            }
        }
        node_cluster.push(merged);
    }

    // Relabel to consecutive integers starting from 0.
    let mut remap: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
    for label in labels.iter_mut() {
        let next = remap.len();
        *label = *remap.entry(*label).or_insert(next);
    }

    labels
}

/// Dot product of two L2-normalized embeddings (= cosine similarity).
fn dot_product(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// The least-restrictive visibility across a cluster of entries. Same-visibility clusters
/// consolidate to that class; cross-visibility clusters consolidate to the least restrictive
/// (public > attributed > private), since the fact was already attested in the less-restrictive
/// form. For `Exclude(...)` sets, the intersection is taken.
fn least_restrictive_visibility(entries: &[EntryView]) -> Visibility {
    if entries.is_empty() {
        return Visibility::Public;
    }
    let mut result = entries[0].visibility.clone();
    for entry in &entries[1..] {
        result = merge_visibility(&result, &entry.visibility);
    }
    result
}

/// Merge two visibilities, taking the least restrictive.
fn merge_visibility(a: &Visibility, b: &Visibility) -> Visibility {
    match (a, b) {
        (Visibility::Public, _) | (_, Visibility::Public) => Visibility::Public,
        (Visibility::Attributed, _) | (_, Visibility::Attributed) => Visibility::Attributed,
        (Visibility::PrivateToTeller, Visibility::PrivateToTeller) => Visibility::PrivateToTeller,
        (Visibility::PrivateToTeller, Visibility::Exclude(ids))
        | (Visibility::Exclude(ids), Visibility::PrivateToTeller) => {
            Visibility::Exclude(ids.clone())
        }
        (Visibility::Exclude(a_ids), Visibility::Exclude(b_ids)) => {
            let intersection: BTreeSet<MemoryId> = a_ids & b_ids;
            if intersection.is_empty() {
                Visibility::PrivateToTeller
            } else {
                Visibility::Exclude(intersection)
            }
        }
    }
}

/// The model's synthesis of a cluster.
struct Synthesis {
    consolidated_text: String,
}

/// Call the model to synthesize a consolidated entry from a cluster.
async fn synthesize_cluster(
    engine: &Engine,
    model: &dyn ModelClient,
    recording: &Recording,
    template_body: &str,
    memory_id: MemoryId,
    cluster: &[EntryView],
    existing_links: &[crate::graph::ClassLinkView],
) -> Result<Option<Synthesis>, InstanceError> {
    let entry_lines: Vec<String> = cluster
        .iter()
        .map(|e| {
            format!(
                "- id: {}, text: {:?}, told_by: {:?}, visibility: {}",
                e.entry_id.0,
                e.text,
                e.told_by,
                visibility_label(&e.visibility)
            )
        })
        .collect();
    let link_lines: Vec<String> = {
        let graph = engine.graph.lock();
        existing_links
            .iter()
            .map(|l| {
                let to_name = graph
                    .memory_by_id(l.to)
                    .ok()
                    .flatten()
                    .map(|m| m.name.as_str().to_owned())
                    .unwrap_or_else(|| l.to.0.to_string());
                format!("- {} → {}", l.relation.as_str(), to_name)
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
        .map_err(|e| InstanceError::from(TurnError::Model(e)))?
        .expect_completed();

    let Some(parsed) =
        crate::model::parse_structured::<ConsolidationSynthesis>(&response.completion)
    else {
        return Ok(None);
    };

    Ok(Some(Synthesis {
        consolidated_text: parsed.consolidated_text,
    }))
}

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
    use super::*;

    #[test]
    fn merge_visibility_picks_least_restrictive() {
        assert_eq!(
            merge_visibility(&Visibility::Public, &Visibility::PrivateToTeller),
            Visibility::Public
        );
        assert_eq!(
            merge_visibility(&Visibility::Attributed, &Visibility::PrivateToTeller),
            Visibility::Attributed
        );
        assert_eq!(
            merge_visibility(&Visibility::PrivateToTeller, &Visibility::PrivateToTeller),
            Visibility::PrivateToTeller
        );
    }

    #[test]
    fn merge_visibility_intersects_exclude_sets() {
        let a = Visibility::Exclude([MemoryId::generate()].into_iter().collect());
        let b = Visibility::Exclude([MemoryId::generate()].into_iter().collect());
        let merged = merge_visibility(&a, &b);
        assert_eq!(merged, Visibility::PrivateToTeller);
    }
}
