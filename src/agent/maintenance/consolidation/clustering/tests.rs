//! Tests for the consolidation clustering pass: grouping entries by visibility
//! posture, clustering within each posture level by embedding similarity, and the
//! cross-level tier-2 dedup that folds near-identical entries across levels.

use crate::{
    agent::maintenance::consolidation::clustering::{
        cluster_within, tier1_groups, tier2_absorptions,
    },
    event::{Teller, Visibility},
    graph::{EntryOrigin, EntryView},
    ids::{EntryId, MemoryId},
    model::embed::Embedding,
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
