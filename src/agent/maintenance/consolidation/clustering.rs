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
mod tests {
    use super::*;
    use crate::{
        event::{Teller, Visibility},
        graph::{EntryOrigin, EntryView},
        ids::{EntryId, MemoryId},
        time::Timestamp,
    };

    fn entry(text: &str, told_by: Teller, visibility: Visibility) -> EntryView {
        entry_with_origin(text, told_by, visibility, EntryOrigin::Recorded)
    }

    fn entry_with_origin(
        text: &str,
        told_by: Teller,
        visibility: Visibility,
        origin: EntryOrigin,
    ) -> EntryView {
        EntryView {
            entry_id: EntryId::generate(),
            asserted_at: Timestamp::from_millis(1_000),
            occurred_sort: None,
            occurred_at: None,
            occurred_authored: false,
            text: text.to_owned(),
            told_by,
            told_in: None,
            visibility,
            superseded_by: None,
            retracted_reason: None,
            origin,
            attestations: Vec::new(),
        }
    }

    #[test]
    fn public_entries_group_across_tellers() {
        let alice = MemoryId::generate();
        let bob = MemoryId::generate();
        let entries = vec![
            entry("a", Teller::Participant(alice), Visibility::Public),
            entry("b", Teller::Participant(bob), Visibility::Public),
        ];
        let groups = tier1_groups(&entries);
        assert_eq!(
            groups.len(),
            1,
            "public entries share one group regardless of teller"
        );
        assert_eq!(groups[0].len(), 2);
    }

    #[test]
    fn attributed_entries_group_across_tellers() {
        let alice = MemoryId::generate();
        let bob = MemoryId::generate();
        let entries = vec![
            entry("a", Teller::Participant(alice), Visibility::Attributed),
            entry("b", Teller::Participant(bob), Visibility::Attributed),
        ];
        let groups = tier1_groups(&entries);
        assert_eq!(
            groups.len(),
            1,
            "attributed entries are all-audience, so they co-synthesize across tellers"
        );
        assert_eq!(groups[0].len(), 2);
    }

    #[test]
    fn attributed_and_private_never_share_a_group() {
        let alice = MemoryId::generate();
        // An attributed entry surfaces to everyone; a private one reaches only its teller. They must
        // never co-synthesize, even from the same teller — the private text would widen to all-audience.
        let entries = vec![
            entry("a", Teller::Participant(alice), Visibility::Attributed),
            entry("a", Teller::Participant(alice), Visibility::PrivateToTeller),
        ];
        let groups = tier1_groups(&entries);
        assert_eq!(
            groups.len(),
            2,
            "an attributed and a private entry land in different groups"
        );
    }

    #[test]
    fn private_entries_split_by_teller() {
        let alice = MemoryId::generate();
        let bob = MemoryId::generate();
        // The privacy-correct residual: two confidences of the same fact from different tellers reach
        // incomparable audiences (each only its own teller), so they never co-synthesize — duplication
        // survives rather than one confidence widening or being misattributed.
        let entries = vec![
            entry("a", Teller::Participant(alice), Visibility::PrivateToTeller),
            entry("b", Teller::Participant(bob), Visibility::PrivateToTeller),
        ];
        let groups = tier1_groups(&entries);
        assert_eq!(
            groups.len(),
            2,
            "private entries with different tellers never co-synthesize"
        );
    }

    #[test]
    fn private_and_public_never_share_a_group() {
        let alice = MemoryId::generate();
        let entries = vec![
            entry("a", Teller::Participant(alice), Visibility::PrivateToTeller),
            entry("a", Teller::Participant(alice), Visibility::Public),
        ];
        let groups = tier1_groups(&entries);
        assert_eq!(
            groups.len(),
            2,
            "a private and a public entry land in different groups"
        );
    }

    #[test]
    fn exclude_sets_group_by_exact_set_equality() {
        let alice = MemoryId::generate();
        let x = MemoryId::generate();
        let y = MemoryId::generate();
        let exclude_x = Visibility::Exclude([x].into_iter().collect());
        let exclude_xy = Visibility::Exclude([x, y].into_iter().collect());
        let entries = vec![
            entry("a", Teller::Participant(alice), exclude_x.clone()),
            entry("b", Teller::Participant(alice), exclude_x),
            entry("c", Teller::Participant(alice), exclude_xy),
        ];
        let groups = tier1_groups(&entries);
        assert_eq!(
            groups.len(),
            2,
            "only entries with the identical exclude set group together"
        );
    }

    #[test]
    fn tier2_defers_a_source_whose_only_target_is_itself_absorbed() {
        // A chain: private P is a near-duplicate of attributed A, A of public Q, but P and Q sit
        // below the bar (two ~16-degree hops sum past the threshold). A absorbs into Q; the P-into-A
        // pair must be dropped, not applied — absorbing it would tombstone A carrying P's account,
        // and P was never a near-duplicate of Q, so it correctly stands as its own entry.
        let x = MemoryId::generate();
        let q = entry("launch shipped", Teller::Agent, Visibility::Public);
        let a = entry(
            "the launch went out",
            Teller::Participant(x),
            Visibility::Attributed,
        );
        let p = entry(
            "shipping happened",
            Teller::Participant(MemoryId::generate()),
            Visibility::PrivateToTeller,
        );
        let q_id = q.entry_id;
        let a_id = a.entry_id;
        let entries = vec![q, a, p];
        // Unit vectors at 0, 16, and 32 degrees: adjacent cosines ~0.961 (above 0.95), the ends
        // ~0.848 (below).
        let embeddings: Vec<Embedding> = vec![
            vec![1.0, 0.0],
            vec![0.961_26, 0.275_64],
            vec![0.848_05, 0.529_92],
        ];
        let absorptions = tier2_absorptions(&entries, &embeddings, 0.95);
        assert_eq!(
            absorptions,
            vec![(q_id, vec![a_id])],
            "only the one-hop absorption survives; the chained source defers"
        );
    }

    #[test]
    fn tier2_absorbs_a_private_source_into_a_public_near_duplicate() {
        let alice = MemoryId::generate();
        // Two identical embeddings force a cosine of 1.0, above any threshold.
        let embeddings = vec![vec![1.0, 0.0], vec![1.0, 0.0]];
        let private = entry(
            "secret",
            Teller::Participant(alice),
            Visibility::PrivateToTeller,
        );
        let public = entry("public", Teller::Agent, Visibility::Public);
        let private_id = private.entry_id;
        let public_id = public.entry_id;
        let entries = vec![private, public];

        let plan = tier2_absorptions(&entries, &embeddings, 0.95);
        assert_eq!(plan, vec![(public_id, vec![private_id])]);
    }

    #[test]
    fn tier2_leaves_a_public_source_alone() {
        let alice = MemoryId::generate();
        let embeddings = vec![vec![1.0, 0.0], vec![1.0, 0.0]];
        // Both entries are all-audience, so neither is a private source to retire.
        let entries = vec![
            entry("a", Teller::Participant(alice), Visibility::Public),
            entry("b", Teller::Agent, Visibility::Public),
        ];
        assert!(tier2_absorptions(&entries, &embeddings, 0.95).is_empty());
    }

    #[test]
    fn tier2_leaves_a_below_threshold_pair_live() {
        let alice = MemoryId::generate();
        // Cosine 0.9 — a near-duplicate at the consolidation bar, but below the stricter 0.95 dedup
        // bar, so the private copy stays live rather than being retired against a merely-similar public.
        let embeddings = vec![vec![1.0, 0.0], vec![0.9, (1.0f32 - 0.81).sqrt()]];
        let entries = vec![
            entry(
                "secret",
                Teller::Participant(alice),
                Visibility::PrivateToTeller,
            ),
            entry("public", Teller::Agent, Visibility::Public),
        ];
        assert!(tier2_absorptions(&entries, &embeddings, 0.95).is_empty());
    }

    #[test]
    fn tier2_absorbs_an_exclude_source_into_a_public_superset() {
        let alice = MemoryId::generate();
        let excluded = MemoryId::generate();
        let embeddings = vec![vec![1.0, 0.0], vec![1.0, 0.0]];
        // An exclude entry (audience: everyone but `excluded`, while alice is present) is a subset of a
        // public entry's audience (everyone), so the public entry is a valid superset replacement.
        let exclude = Visibility::Exclude([excluded].into_iter().collect());
        let source = entry("secret", Teller::Participant(alice), exclude);
        let public = entry("public", Teller::Agent, Visibility::Public);
        let source_id = source.entry_id;
        let public_id = public.entry_id;
        let entries = vec![source, public];
        assert_eq!(
            tier2_absorptions(&entries, &embeddings, 0.95),
            vec![(public_id, vec![source_id])]
        );
    }

    #[test]
    fn tier2_absorbs_an_attributed_source_into_a_public_entry() {
        let alice = MemoryId::generate();
        let embeddings = vec![vec![1.0, 0.0], vec![1.0, 0.0]];
        // An attributed entry is all-audience but attribution-bearing. Folding it into a plain public
        // near-duplicate is sound — both surface to everyone — and the attribution survives as an
        // Attributed attestation the write path leaves on the public entry.
        let attributed = entry(
            "attributed",
            Teller::Participant(alice),
            Visibility::Attributed,
        );
        let public = entry("public", Teller::Agent, Visibility::Public);
        let attributed_id = attributed.entry_id;
        let public_id = public.entry_id;
        let entries = vec![attributed, public];
        assert_eq!(
            tier2_absorptions(&entries, &embeddings, 0.95),
            vec![(public_id, vec![attributed_id])]
        );
    }

    #[test]
    fn tier2_never_absorbs_an_attributed_source_into_an_attributed_target() {
        let alice = MemoryId::generate();
        let bob = MemoryId::generate();
        let embeddings = vec![vec![1.0, 0.0], vec![1.0, 0.0]];
        // An attributed source folds only into a plain public target, never another attributed one:
        // the target carries its own teller's attribution, and collapsing two attributed accounts would
        // muddy whose attribution stands. Neither is retired.
        let entries = vec![
            entry("a", Teller::Participant(alice), Visibility::Attributed),
            entry("b", Teller::Participant(bob), Visibility::Attributed),
        ];
        assert!(tier2_absorptions(&entries, &embeddings, 0.95).is_empty());
    }

    #[test]
    fn tier2_never_absorbs_a_private_source_into_a_private_target() {
        let alice = MemoryId::generate();
        let bob = MemoryId::generate();
        let embeddings = vec![vec![1.0, 0.0], vec![1.0, 0.0]];
        // A near-duplicate private pair told by different tellers: no all-audience target exists, so
        // neither is retired — a private fact is never folded into another private one.
        let entries = vec![
            entry("a", Teller::Participant(alice), Visibility::PrivateToTeller),
            entry("b", Teller::Participant(bob), Visibility::PrivateToTeller),
        ];
        assert!(tier2_absorptions(&entries, &embeddings, 0.95).is_empty());
    }

    #[test]
    fn tier1_never_groups_a_connector_maintained_entry() {
        let alice = MemoryId::generate();
        // Two public entries that would ordinarily share a group, but one is connector-maintained —
        // so it is dropped from grouping entirely and the surviving group holds only the recorded one.
        let entries = vec![
            entry_with_origin(
                "username: alice",
                Teller::Agent,
                Visibility::Public,
                EntryOrigin::PlatformConnector("discord".to_owned()),
            ),
            entry(
                "a genuine fact",
                Teller::Participant(alice),
                Visibility::Public,
            ),
        ];
        let groups = tier1_groups(&entries);
        assert_eq!(groups.len(), 1, "the connector entry forms no group");
        assert_eq!(
            groups[0],
            vec![1],
            "only the recorded entry (index 1) is grouped"
        );
    }

    #[test]
    fn tier2_never_absorbs_a_connector_maintained_source() {
        let alice = MemoryId::generate();
        let embeddings = vec![vec![1.0, 0.0], vec![1.0, 0.0]];
        // A connector-maintained private-looking entry is never a source, even against a public
        // near-duplicate: the connector owns it and may retract it at any time.
        let entries = vec![
            entry_with_origin(
                "nickname",
                Teller::Participant(alice),
                Visibility::PrivateToTeller,
                EntryOrigin::PlatformConnector("discord".to_owned()),
            ),
            entry("public", Teller::Agent, Visibility::Public),
        ];
        assert!(tier2_absorptions(&entries, &embeddings, 0.95).is_empty());
    }

    #[test]
    fn tier2_never_absorbs_into_a_connector_maintained_target() {
        let alice = MemoryId::generate();
        let embeddings = vec![vec![1.0, 0.0], vec![1.0, 0.0]];
        // The only all-audience near-duplicate is connector-maintained, so it is not a valid
        // replacement target and the private source stays live rather than pointing at an entry the
        // connector may supersede out from under it.
        let entries = vec![
            entry(
                "secret",
                Teller::Participant(alice),
                Visibility::PrivateToTeller,
            ),
            entry_with_origin(
                "display name",
                Teller::Agent,
                Visibility::Public,
                EntryOrigin::PlatformConnector("discord".to_owned()),
            ),
        ];
        assert!(tier2_absorptions(&entries, &embeddings, 0.95).is_empty());
    }

    #[test]
    fn a_dissimilar_pair_stays_unclustered_at_the_cut() {
        // Cosine 0.0 — far below any sane threshold. The cut is over dissimilarities, so this
        // guards the similarity-to-dissimilarity inversion: an inverted cut merges everything with
        // cosine above 1 - threshold, which this pair would satisfy.
        let embeddings: Vec<Embedding> = vec![vec![1.0, 0.0], vec![0.0, 1.0]];
        let clusters = cluster_within(&embeddings, &[0, 1], 0.85);
        assert_eq!(
            clusters,
            vec![vec![0], vec![1]],
            "a dissimilar pair must stay two singletons"
        );
    }

    #[test]
    fn a_similar_pair_clusters_while_a_dissimilar_third_stays_out() {
        // Indices 0 and 1 are near-identical (cosine ~0.995); index 2 is orthogonal.
        let embeddings: Vec<Embedding> = vec![vec![1.0, 0.0], vec![0.995, 0.0999], vec![0.0, 1.0]];
        let clusters = cluster_within(&embeddings, &[0, 1, 2], 0.85);
        assert!(
            clusters.contains(&vec![0, 1]),
            "the near-identical pair clusters: {clusters:?}"
        );
        assert!(
            clusters.contains(&vec![2]),
            "the orthogonal entry stays a singleton: {clusters:?}"
        );
    }
}
