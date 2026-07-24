//! Embedding, posture grouping, within-level clustering, and cross-level dedup — the structural
//! machinery the consolidation pass runs before (tier 1) and instead of (tier 2) a synthesis call.

use std::collections::BTreeSet;

use crate::{
    IndexError, InstanceError,
    agent::TurnError,
    engine::Engine,
    event::{Teller, Visibility},
    graph::EntryView,
    ids::{EntryId, MemoryId},
    model::{embed::Embedding, index::VectorKey},
    vector::VectorRecord,
};

/// Embed a class's live entries and return their contextual embeddings, aligned to `entries`. Only
/// the entries missing from the vector index are embedded: the index is GC'd on supersede, retraction,
/// and consolidation, so a present key is a live, current vector that is read back rather than paid for
/// again. The raw `Entry` vector (search) and the `EntryContextual` vector (clustering and dedup) are
/// both upserted for a genuinely missing entry, in one embed batch to avoid a double round-trip.
/// Returns an empty vector when the instance has no retrieval attached.
pub(super) async fn embed_class_entries(
    engine: &Engine,
    memory_id: MemoryId,
    entries: &[EntryView],
) -> Result<Vec<Embedding>, InstanceError> {
    let Some(retrieval) = &engine.retrieval else {
        return Ok(Vec::new());
    };

    // The memory name is the contextual embedding's prefix: it normalizes name-bearing and name-less
    // phrasings so the same fact clusters together regardless of whether the entry repeats the subject.
    let memory_name = {
        let graph = engine.graph.lock();
        graph
            .memory_by_id(memory_id)
            .ok()
            .flatten()
            .map(|memory| memory.name)
    };

    // Read back the vectors already indexed. The contextual vector is needed by value (for clustering);
    // the raw vector is needed only by presence (whether to re-index it for search).
    let (existing_contextual, entry_present): (Vec<Option<Embedding>>, Vec<bool>) = {
        let vectors = retrieval.vectors.lock();
        let mut contextual = Vec::with_capacity(entries.len());
        let mut raw_present = Vec::with_capacity(entries.len());
        for entry in entries {
            contextual.push(
                vectors
                    .get(&VectorKey::EntryContextual(entry.entry_id).to_vector_id())
                    .map_err(IndexError::Vector)?,
            );
            raw_present.push(
                vectors
                    .get(&VectorKey::Entry(entry.entry_id).to_vector_id())
                    .map_err(IndexError::Vector)?
                    .is_some(),
            );
        }
        (contextual, raw_present)
    };

    // Collect the missing texts to embed: raw first, then contextual, so one batch splits cleanly.
    let mut to_embed: Vec<String> = Vec::new();
    let mut raw_targets: Vec<usize> = Vec::new();
    for (i, entry) in entries.iter().enumerate() {
        if !entry_present[i] {
            to_embed.push(entry.text.clone());
            raw_targets.push(i);
        }
    }
    let raw_count = to_embed.len();
    let mut contextual_targets: Vec<usize> = Vec::new();
    for (i, entry) in entries.iter().enumerate() {
        if existing_contextual[i].is_none() {
            to_embed.push(match &memory_name {
                Some(name) => crate::model::embed::contextual_text(name.as_str(), &entry.text),
                None => entry.text.clone(),
            });
            contextual_targets.push(i);
        }
    }

    let fresh = if to_embed.is_empty() {
        Vec::new()
    } else {
        retrieval
            .embedder
            .embed(&to_embed)
            .await
            .map_err(|error| InstanceError::from(TurnError::Model(error)))?
    };
    let (raw_fresh, contextual_fresh) = fresh.split_at(raw_count);

    // Upsert the freshly embedded vectors into both spaces.
    {
        let mut vectors = retrieval.vectors.lock();
        let model_id = retrieval.embedder.model_id();
        for (slot, &i) in raw_targets.iter().enumerate() {
            vectors
                .upsert(VectorRecord {
                    id: VectorKey::Entry(entries[i].entry_id).to_vector_id(),
                    embedding: raw_fresh[slot].clone(),
                    model_id: model_id.into(),
                })
                .map_err(IndexError::Vector)?;
        }
        for (slot, &i) in contextual_targets.iter().enumerate() {
            vectors
                .upsert(VectorRecord {
                    id: VectorKey::EntryContextual(entries[i].entry_id).to_vector_id(),
                    embedding: contextual_fresh[slot].clone(),
                    model_id: model_id.into(),
                })
                .map_err(IndexError::Vector)?;
        }
    }

    // Assemble the full contextual set in entry order: reuse the indexed vectors, fill the gaps in
    // order with the fresh ones.
    let mut fresh_contextual = contextual_fresh.iter();
    let mut contextual = Vec::with_capacity(entries.len());
    for existing in existing_contextual {
        match existing {
            Some(embedding) => contextual.push(embedding),
            None => contextual.push(
                fresh_contextual
                    .next()
                    .expect("one fresh contextual vector for each missing entry")
                    .clone(),
            ),
        }
    }
    Ok(contextual)
}

/// Group a class's live entries into tier-1 synthesis groups by visibility posture, as entry indices.
/// A group is the unit within which clustering and synthesis run, so the grouping fixes the audience
/// of every synthesized replacement: [`Visibility::Public`] and [`Visibility::Attributed`] entries
/// merge across tellers (both surface to everyone, so synthesizing two relayed accounts leaks nothing
/// — the tellers survive as attestations on the replacement), while [`Visibility::PrivateToTeller`] and
/// [`Visibility::Exclude`] entries group per teller (and per exact exclude set). That per-teller split
/// is the deliberate privacy-correct residual where duplication survives: a synthesized text interleaves
/// its sources' clauses, and two confidences' audiences are incomparable (each reaches only its own
/// teller, or all-but-its-own-excluded-set), so merging them would either widen one confidence's
/// audience or attribute it to a teller who never told it. Keeping them apart keeps a private confidence
/// from being synthesized into a copy visible to, or attributed to, anyone but its own teller.
pub(super) fn tier1_groups(entries: &[EntryView]) -> Vec<Vec<usize>> {
    let mut groups: Vec<(PostureKey, Vec<usize>)> = Vec::new();
    for (i, entry) in entries.iter().enumerate() {
        // A connector-maintained entry is never grouped, so it can never be a consolidation source:
        // the connector owns its id and supersedes or retracts it as the platform-side account
        // changes, and folding it into a synthesized replacement would strand that maintenance.
        if entry.origin.is_connector() {
            continue;
        }
        let key = posture_key(entry);
        match groups.iter_mut().find(|(existing, _)| *existing == key) {
            Some((_, members)) => members.push(i),
            None => groups.push((key, vec![i])),
        }
    }
    groups.into_iter().map(|(_, members)| members).collect()
}

/// Cluster a group's entries by cosine similarity using complete linkage at `threshold`, returning
/// clusters of original entry indices. Singletons are included so the caller can skip them. Operates on
/// the precomputed contextual `embeddings` indexed by the group's `indices`, so the whole class embeds
/// once and every group clusters against that single set.
pub(super) fn cluster_within(
    embeddings: &[Embedding],
    indices: &[usize],
    threshold: f64,
) -> Vec<Vec<usize>> {
    let m = indices.len();
    if m < 2 {
        return indices.iter().map(|&i| vec![i]).collect();
    }

    let mut dissimilarities: Vec<f32> = Vec::with_capacity(m * (m - 1) / 2);
    for a in 0..m {
        for b in (a + 1)..m {
            let sim = dot_product(&embeddings[indices[a]], &embeddings[indices[b]]);
            dissimilarities.push(1.0 - sim);
        }
    }

    let dendrogram = kodama::linkage(&mut dissimilarities, m, kodama::Method::Complete);
    // The dendrogram is built over dissimilarities (1 - cosine), so the similarity threshold
    // inverts at the cut: a cluster merges only while its complete-linkage dissimilarity stays
    // within 1 - threshold, which is exactly "every pair's cosine is at least the threshold".
    let labels = cut_tree(dendrogram.steps(), m, (1.0 - threshold) as f32);

    let mut clusters: Vec<Vec<usize>> = Vec::new();
    let mut label_to_slot: Vec<Option<usize>> = vec![None; m];
    for (local, &label) in labels.iter().enumerate() {
        let original = indices[local];
        match label_to_slot.get(label).and_then(|slot| *slot) {
            Some(slot) => clusters[slot].push(original),
            None => {
                let slot = clusters.len();
                label_to_slot[label] = Some(slot);
                clusters.push(vec![original]);
            }
        }
    }
    clusters
}

/// The tier-2 cross-level dedup plan: an entry whose fact is already attested by a wider-or-equal
/// entry (cosine ≥ the stricter `threshold`) is retired into that entry, its own teller surviving as an
/// attestation the write path leaves on the replacement. Returns each retained replacement entry paired
/// with the source entries to fold into it.
///
/// A source is eligible for a target when its posture is strictly narrower than, or attribution-
/// preserving under, the target's (see [`is_absorbable`]): a [`Visibility::PrivateToTeller`] or
/// [`Visibility::Exclude`] confidence folds into any all-audience entry, and a [`Visibility::Attributed`]
/// entry folds into a plain [`Visibility::Public`] one — the attribution survives as an `Attributed`
/// attestation on the public entry, so no audience is rotated or narrowed and nothing leaks. A
/// [`Visibility::Public`] entry is never a source (it is already the widest audience), and a private
/// entry is never a replacement, so a fold only ever collapses a narrower or equally-wide copy into an
/// at-least-as-wide one. Among qualifying replacements the most public, then most similar, wins.
pub(super) fn tier2_absorptions(
    entries: &[EntryView],
    embeddings: &[Embedding],
    threshold: f64,
) -> Vec<(EntryId, Vec<EntryId>)> {
    let threshold = threshold as f32;
    let mut by_target: Vec<(usize, Vec<EntryId>)> = Vec::new();
    for (i, entry) in entries.iter().enumerate() {
        // A connector-maintained entry is excluded from both roles: never a source (the connector
        // owns it) and — handled in the candidate loop below — never a replacement target (an
        // absorbed entry must not point its `superseded_by` at an entry the connector may supersede
        // out from under it). Excluding it from both keeps the cleanup off connector-owned records
        // entirely.
        if entry.origin.is_connector() || matches!(entry.visibility, Visibility::Public) {
            continue;
        }
        let mut best: Option<(usize, bool, f32)> = None;
        for (j, candidate) in entries.iter().enumerate() {
            if j == i
                || candidate.origin.is_connector()
                || !is_absorbable(&entry.visibility, &candidate.visibility)
            {
                continue;
            }
            let score = dot_product(&embeddings[i], &embeddings[j]);
            if score < threshold {
                continue;
            }
            let is_public = matches!(candidate.visibility, Visibility::Public);
            let better = match best {
                None => true,
                Some((_, best_public, best_score)) => {
                    (is_public, score) > (best_public, best_score)
                }
            };
            if better {
                best = Some((j, is_public, score));
            }
        }
        if let Some((target, _, _)) = best {
            match by_target
                .iter_mut()
                .find(|(existing, _)| *existing == target)
            {
                Some((_, sources)) => sources.push(entry.entry_id),
                None => by_target.push((target, vec![entry.entry_id])),
            }
        }
    }
    // An entry absorbed this sweep must not also serve as a target: a chained absorption (private P
    // into attributed A, A into public Q, with P and Q below the bar) would tombstone A while it
    // still carries P's just-planned attestation, silently losing P's account. Dropping the pair
    // whose target is itself absorbed keeps every absorption one hop per sweep — and the stranded
    // source is not merely deferred, it is correctly refused: P was never a near-duplicate of Q, so
    // once A is gone it stands as its own entry. The screen is over the original pick's source set,
    // which can over-defer a pair whose source's own absorption was dropped; that entry is simply
    // reconsidered on a later sweep.
    let absorbed: BTreeSet<EntryId> = by_target
        .iter()
        .flat_map(|(_, sources)| sources.iter().copied())
        .collect();
    by_target
        .into_iter()
        .filter(|(target, _)| !absorbed.contains(&entries[*target].entry_id))
        .map(|(target, sources)| (entries[target].entry_id, sources))
        .collect()
}

/// The posture that fixes an entry's tier-1 group. Public and attributed entries each share one key
/// regardless of teller (both surface to everyone); the private and exclude postures key on the teller
/// (and, for an exclude, the exact withheld set), since below the all-audience tier the teller
/// determines who may see the fact.
#[derive(PartialEq, Eq)]
enum PostureKey {
    Public,
    Attributed,
    PrivateToTeller(Teller),
    Exclude(Teller, BTreeSet<MemoryId>),
}

fn posture_key(entry: &EntryView) -> PostureKey {
    match &entry.visibility {
        Visibility::Public => PostureKey::Public,
        Visibility::Attributed => PostureKey::Attributed,
        Visibility::PrivateToTeller => PostureKey::PrivateToTeller(entry.told_by.clone()),
        Visibility::Exclude(set) => PostureKey::Exclude(entry.told_by.clone(), set.clone()),
    }
}

/// Whether a `source` entry may be retired into a `target` entry — the tier-2 eligibility rule. A fold
/// is sound exactly when the target's audience is a superset of the source's, or the two are equally
/// wide but the fold preserves the source's attribution as an attestation:
///
/// - a private or exclude confidence into any all-audience entry (the classic narrower-into-wider case);
/// - an attributed entry into a plain public one (equally wide, but the attribution survives as an
///   `Attributed` attestation the write path leaves on the public entry).
///
/// Never a [`Visibility::Public`] source (already the widest), and never a private target, so a fold
/// never rotates or narrows an audience.
fn is_absorbable(source: &Visibility, target: &Visibility) -> bool {
    matches!(
        (source, target),
        (
            Visibility::PrivateToTeller | Visibility::Exclude(_),
            Visibility::Public | Visibility::Attributed,
        ) | (Visibility::Attributed, Visibility::Public)
    )
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

#[cfg(test)]
mod tests;
