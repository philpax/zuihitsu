//! The relation-vocabulary view: which typed edges the runs drew, whether each was seeded at genesis
//! or coined after, and the namespace shapes each links.

use std::collections::{BTreeMap, BTreeSet};

use zuihitsu::{EventPayload, MemoryId};

use crate::package::EvalPackage;

use crate::analyze::format::{plural, render_locations};

/// How many namespace shapes to show per relation before collapsing the tail into a `+N more` note —
/// enough to read the dominant shapes at a glance without letting a scattered relation run off the line.
const MAX_SHAPES: usize = 6;

/// The relation-vocabulary view: which typed edges the runs drew, whether each was seeded at genesis or
/// coined after, and the namespace shapes each links. This is the promotion of a projection three
/// separate sweeps hand-rolled — build an id→name map per run, then read every `LinkCreated` as
/// `from_namespace → to_namespace` cross-tabulated by relation — the canonical example the analysis skill
/// names as earning a place in `analyze`. `scenario` restricts the scan to scenarios whose name contains
/// the substring.
pub(crate) fn print_relations(pkg: &EvalPackage, scenario: Option<&str>) {
    let report = project_relations(pkg, scenario);

    println!(
        "\n=== relation vocabulary — {} relation{} used across {} run{} ===\n",
        report.vocab.len(),
        plural(report.vocab.len()),
        report.runs_scanned,
        plural(report.runs_scanned),
    );
    if report.vocab.is_empty() {
        println!("  none");
    } else {
        let width = report
            .vocab
            .iter()
            .map(|row| row.relation.len())
            .max()
            .unwrap_or(8)
            .max("relation".len());
        println!(
            "  {:width$}  {:>4}  {:>5}  shapes",
            "relation", "seed", "uses"
        );
        for row in &report.vocab {
            println!(
                "  {:width$}  {:>4}  {:>5}  {}",
                row.relation,
                if row.seeded { "yes" } else { "no" },
                row.uses,
                render_shapes(&row.shapes),
            );
        }
    }

    println!(
        "\n=== coined relations — {} registered outside genesis ===\n",
        report.coinages.len(),
    );
    if report.coinages.is_empty() {
        println!("  none");
        return;
    }
    for coinage in &report.coinages {
        println!(
            "  {} (inverse: {}) — coined in {} run{}, {} link{}",
            coinage.relation,
            coinage.inverse,
            coinage.coined_in_runs,
            plural(coinage.coined_in_runs),
            coinage.uses,
            plural(coinage.uses),
        );
        println!("    runs: {}", render_locations(&coinage.locations));
    }
}

/// The whole-package relation projection: the vocabulary rows (sorted most-used first) and the coinages
/// (relations registered outside genesis, sorted most-used first). Split out from the rendering so the
/// tabulation is unit-testable on a synthetic package.
pub(crate) fn project_relations(pkg: &EvalPackage, scenario: Option<&str>) -> RelationsReport {
    // A relation label counts as seeded if it (or its inverse) was registered before its run's
    // `GenesisCompleted` marker — the genesis rollout emits the seed relations ahead of it, so position
    // relative to that event is the source signal, derived from the package rather than a hardcoded list
    // that would drift as the seeded set changes.
    let mut seeded: BTreeSet<String> = BTreeSet::new();
    let mut uses: BTreeMap<String, usize> = BTreeMap::new();
    let mut shapes: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
    let mut coined: BTreeMap<String, CoinedAcc> = BTreeMap::new();
    let mut runs_scanned = 0usize;

    for report in &pkg.scenarios {
        if scenario.is_some_and(|sub| !report.meta.name.contains(sub)) {
            continue;
        }
        for run in &report.runs {
            runs_scanned += 1;
            let events = &run.events;
            // The genesis boundary: everything before the first `GenesisCompleted` is the seed rollout.
            // A run with no marker (a synthetic fixture) treats every registration as post-genesis.
            let genesis_at = events
                .iter()
                .position(|event| matches!(event.payload, EventPayload::GenesisCompleted { .. }))
                .unwrap_or(0);

            let mut names: BTreeMap<MemoryId, String> = BTreeMap::new();
            for event in events {
                if let EventPayload::MemoryCreated { id, name } = &event.payload {
                    names.insert(*id, name.as_str().to_owned());
                }
            }

            for (index, event) in events.iter().enumerate() {
                if let EventPayload::LinkTypeRegistered { name, inverse, .. } = &event.payload {
                    if index < genesis_at {
                        seeded.insert(name.as_str().to_owned());
                        seeded.insert(inverse.as_str().to_owned());
                    } else {
                        coined
                            .entry(name.as_str().to_owned())
                            .or_insert_with(|| CoinedAcc {
                                inverse: inverse.as_str().to_owned(),
                                locations: Vec::new(),
                            })
                            .locations
                            .push((report.meta.name.clone(), run.index));
                    }
                }
            }

            for event in events {
                if let EventPayload::LinkCreated {
                    from, to, relation, ..
                } = &event.payload
                {
                    let relation = relation.as_str().to_owned();
                    *uses.entry(relation.clone()).or_default() += 1;
                    let shape = format!(
                        "{}→{}",
                        namespace_of(&names, from),
                        namespace_of(&names, to)
                    );
                    *shapes
                        .entry(relation)
                        .or_default()
                        .entry(shape)
                        .or_default() += 1;
                }
            }
        }
    }

    let mut vocab: Vec<VocabRow> = uses
        .iter()
        .map(|(relation, &count)| {
            let mut shape_rows: Vec<(String, usize)> = shapes
                .get(relation)
                .map(|counts| counts.iter().map(|(s, &c)| (s.clone(), c)).collect())
                .unwrap_or_default();
            // Most-frequent shape first, ties broken by name for a stable order.
            shape_rows.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            VocabRow {
                relation: relation.clone(),
                seeded: seeded.contains(relation),
                uses: count,
                shapes: shape_rows,
            }
        })
        .collect();
    vocab.sort_by(|a, b| {
        b.uses
            .cmp(&a.uses)
            .then_with(|| a.relation.cmp(&b.relation))
    });

    let mut coinages: Vec<Coinage> = coined
        .into_iter()
        .map(|(relation, acc)| {
            let mut locations = acc.locations;
            locations.sort();
            locations.dedup();
            // The coined relation's link uses under either label — the drift magnitude the sweeps chase.
            let mut link_uses = uses.get(&relation).copied().unwrap_or(0);
            if acc.inverse != relation {
                link_uses += uses.get(&acc.inverse).copied().unwrap_or(0);
            }
            Coinage {
                relation,
                inverse: acc.inverse,
                coined_in_runs: locations.len(),
                uses: link_uses,
                locations,
            }
        })
        .collect();
    coinages.sort_by(|a, b| {
        b.uses
            .cmp(&a.uses)
            .then_with(|| a.relation.cmp(&b.relation))
    });

    RelationsReport {
        runs_scanned,
        vocab,
        coinages,
    }
}

/// The relation projection over a whole package.
pub(crate) struct RelationsReport {
    pub(crate) runs_scanned: usize,
    pub(crate) vocab: Vec<VocabRow>,
    pub(crate) coinages: Vec<Coinage>,
}

/// One relation's row in the vocabulary table.
pub(crate) struct VocabRow {
    pub(crate) relation: String,
    pub(crate) seeded: bool,
    pub(crate) uses: usize,
    /// The namespace shapes this relation linked, e.g. `("person→person", 37)`, most-frequent first.
    pub(crate) shapes: Vec<(String, usize)>,
}

/// One relation registered outside genesis — the coinage signal.
pub(crate) struct Coinage {
    pub(crate) relation: String,
    pub(crate) inverse: String,
    pub(crate) coined_in_runs: usize,
    pub(crate) uses: usize,
    /// The `(scenario, run index)` pairs the relation was coined in, sorted and deduplicated.
    pub(crate) locations: Vec<(String, u32)>,
}

/// A coined relation accumulating its inverse and the runs it appeared in, before deduplication.
struct CoinedAcc {
    inverse: String,
    locations: Vec<(String, u32)>,
}

/// The namespace of a memory endpoint: the reserved `self` handle stands alone, an unresolvable id (no
/// `MemoryCreated` in the run) renders as a short id stub, and any other handle's namespace is whatever
/// precedes its first `/` (`person/marcus` → `person`, `context/chat:room` → `context`).
fn namespace_of(names: &BTreeMap<MemoryId, String>, id: &MemoryId) -> String {
    let Some(name) = names.get(id) else {
        return id.0.to_string().chars().take(8).collect();
    };
    if name == "self" {
        return "self".to_owned();
    }
    match name.split_once('/') {
        Some((prefix, _)) => prefix.to_owned(),
        None => name.clone(),
    }
}

/// Render a relation's namespace shapes as `person→person ×37, event→topic ×233`, capping at
/// [`MAX_SHAPES`] and collapsing the remainder into a `+N more` note so a scattered relation stays on one
/// line.
pub(crate) fn render_shapes(shapes: &[(String, usize)]) -> String {
    if shapes.is_empty() {
        return "—".to_owned();
    }
    let mut rendered: Vec<String> = shapes
        .iter()
        .take(MAX_SHAPES)
        .map(|(shape, count)| format!("{shape} ×{count}"))
        .collect();
    if shapes.len() > MAX_SHAPES {
        rendered.push(format!("+{} more", shapes.len() - MAX_SHAPES));
    }
    rendered.join(", ")
}
