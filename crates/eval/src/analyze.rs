//! `eval analyze` — read an eval package at the terminal. The console renders a package richly, but to
//! judge a run and decide the next prompt/code edit you often just want two things at a prompt: how each
//! scenario moved against a baseline, and the *complete deliberation* of the runs that failed — the
//! agent's per-step reasoning, the Lua it emitted with its results, and which oracle it missed and why.
//! This is the command-line counterpart to the viewer, typed directly against the package contract.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

use zuihitsu::{EventPayload, MemoryId};

use crate::{
    error::EvalError,
    package::{Bar, EvalPackage, RunRecord, ScenarioReport},
};

/// The parameters of an `analyze` invocation: the package to read, an optional baseline, which view to
/// render, and the filters that view honors. Bundled into one request rather than threaded as positional
/// arguments — the CLI's `Analyze` subcommand maps its flags straight onto these fields.
pub struct AnalyzeRequest<'a> {
    pub package: &'a Path,
    pub baseline: Option<&'a Path>,
    /// Dump the failed runs' deliberation traces instead of the summary.
    pub failures: bool,
    /// Render the relation-vocabulary projection instead of the summary. Takes precedence over
    /// `failures` when both are set.
    pub relations: bool,
    /// Restrict every view to scenarios whose name contains this substring.
    pub scenario: Option<&'a str>,
    /// With `failures`, also summarize the events whose payload type contains this substring.
    pub events: Option<&'a str>,
    /// Cap the failed runs dumped per scenario (`0` = all).
    pub limit: usize,
    /// Clip long reasoning and scripts to this many characters (`0` = full).
    pub truncate: usize,
}

/// Print the summary (the default), or — with `failures` — the failed runs: a cross-scenario rollup of
/// every missed verdict (the "what to work on" view), then each failed run's complete deliberation
/// trace; or — with `relations` — the relation-vocabulary projection (which relations were used, whether
/// each was seeded at genesis, the namespace shapes they link, and which were coined outside genesis).
/// `scenario` restricts every mode to scenarios whose name contains the substring; `limit` caps the
/// failed runs dumped per scenario (`0` = all); `truncate` clips long reasoning/scripts (`0` = full).
/// `events` adds, to each dumped run, the events whose payload type contains the substring (e.g.
/// `Scheduled`, `ContentAppended`, `TemporalResolved`) with a compact field summary — the per-run
/// diagnostic that pinpoints *why* a run failed at the event level.
pub fn analyze(request: AnalyzeRequest) -> Result<(), EvalError> {
    let AnalyzeRequest {
        package,
        baseline,
        failures,
        relations,
        scenario,
        events,
        limit,
        truncate,
    } = request;
    let pkg = load(package)?;
    if relations {
        print_relations(&pkg, scenario);
    } else if failures {
        print_failures(&pkg, scenario, events, limit, truncate);
    } else {
        let base = baseline.map(load).transpose()?;
        print_summary(&pkg, base.as_ref(), scenario);
    }
    Ok(())
}

fn load(path: &Path) -> Result<EvalPackage, EvalError> {
    let text = fs::read_to_string(path).map_err(|source| EvalError::ReadPackage {
        path: path.to_path_buf(),
        source,
    })?;
    serde_json::from_str(&text).map_err(|source| EvalError::LoadPackage {
        path: path.to_path_buf(),
        source: Box::new(source),
    })
}

fn bar_label(bar: &Bar) -> String {
    match bar {
        Bar::Gating => "gate".to_owned(),
        Bar::RateGate { threshold } => format!("gate>={threshold}"),
        Bar::Metric { threshold } => format!(">={threshold}"),
    }
}

/// Whether a scenario's aggregate clears its bar — a held gate, a rate at or above a rate gate's
/// threshold, or a metric rate at or above its reporting threshold.
fn clears_bar(report: &ScenarioReport) -> bool {
    match report.meta.bar {
        Bar::Gating => report.aggregate.gating_passed,
        Bar::RateGate { threshold } | Bar::Metric { threshold } => {
            report.aggregate.rate >= threshold
        }
    }
}

fn print_summary(pkg: &EvalPackage, base: Option<&EvalPackage>, scenario: Option<&str>) {
    let base_rate = |name: &str| {
        base.and_then(|b| b.scenarios.iter().find(|s| s.meta.name == name))
            .map(|s| s.aggregate.rate)
    };

    let mut reports: Vec<&ScenarioReport> = pkg
        .scenarios
        .iter()
        .filter(|s| scenario.is_none_or(|sub| s.meta.name.contains(sub)))
        .collect();
    reports.sort_by(|a, b| a.meta.name.cmp(&b.meta.name));

    let perfect = pkg
        .scenarios
        .iter()
        .filter(|s| s.aggregate.rate == 1.0 && s.aggregate.gating_passed)
        .count();
    println!(
        "{} scenarios, {perfect} perfect (1.0 + gate held){}\n",
        pkg.scenarios.len(),
        base.map_or(String::new(), |_| " — Δ vs baseline".to_owned()),
    );

    let width = reports.iter().map(|s| s.meta.name.len()).max().unwrap_or(8);
    println!(
        "{:width$}  {:>7}  {:>5}  {:>5}{}",
        "scenario",
        "bar",
        "rate",
        "gate",
        if base.is_some() {
            "   base       Δ"
        } else {
            ""
        },
    );
    for r in &reports {
        let a = &r.aggregate;
        print!(
            "{:width$}  {:>7}  {:5.2}  {:>5}",
            r.meta.name,
            bar_label(&r.meta.bar),
            a.rate,
            if a.gating_passed { "ok" } else { "FAIL" },
        );
        if base.is_some() {
            match base_rate(&r.meta.name) {
                Some(b) => print!("   {b:5.2}  {:+6.2}", a.rate - b),
                None => print!("   {:>5}  {:>6}", "-", "-"),
            }
        }
        if !clears_bar(r) {
            print!("   <-- BELOW BAR");
        }
        println!();
    }

    let gate_fail: Vec<&str> = reports
        .iter()
        .filter(|r| !r.aggregate.gating_passed)
        .map(|r| r.meta.name.as_str())
        .collect();
    let below: Vec<&str> = reports
        .iter()
        .filter(|r| !clears_bar(r))
        .map(|r| r.meta.name.as_str())
        .collect();
    println!("\ngating not held: {}", join_or_none(&gate_fail));
    println!("below bar:       {}", join_or_none(&below));

    if base.is_some() {
        let mut reg: Vec<String> = Vec::new();
        let mut imp: Vec<String> = Vec::new();
        for r in &reports {
            if let Some(b) = base_rate(&r.meta.name) {
                let delta = r.aggregate.rate - b;
                if delta <= -0.10 {
                    reg.push(format!("{} {b:.2}->{:.2}", r.meta.name, r.aggregate.rate));
                } else if delta >= 0.10 {
                    imp.push(format!("{} {b:.2}->{:.2}", r.meta.name, r.aggregate.rate));
                }
            }
        }
        println!(
            "regressed >=0.10: {}",
            if reg.is_empty() {
                "none".to_owned()
            } else {
                reg.join(", ")
            },
        );
        println!(
            "improved  >=0.10: {}",
            if imp.is_empty() {
                "none".to_owned()
            } else {
                imp.join(", ")
            },
        );
    }
}

/// Every missed verdict across the (filtered) suite, rolled up by criterion — the cross-scenario view
/// that tells you what to work on next. A criterion that slips in several scenarios is one behavioural
/// thread, not N unrelated fails; grouping surfaces that. Each line gives the miss count, the scenarios
/// it appeared in, and one sample rationale. Sorted most-missed first.
fn print_rollup(pkg: &EvalPackage, scenario: Option<&str>) {
    /// One criterion's accumulated misses.
    struct Miss {
        criterion: String,
        count: usize,
        scenarios: Vec<String>,
        sample: String,
    }
    let mut misses: std::collections::BTreeMap<String, Miss> = std::collections::BTreeMap::new();
    for report in &pkg.scenarios {
        if scenario.is_some_and(|sub| !report.meta.name.contains(sub)) {
            continue;
        }
        for run in &report.runs {
            for verdict in &run.verdicts {
                if verdict.passed {
                    continue;
                }
                let entry = misses.entry(verdict.criterion.clone()).or_insert(Miss {
                    criterion: verdict.criterion.clone(),
                    count: 0,
                    scenarios: Vec::new(),
                    sample: verdict.rationale.trim().to_owned(),
                });
                entry.count += 1;
                if !entry.scenarios.iter().any(|s| s == &report.meta.name) {
                    entry.scenarios.push(report.meta.name.clone());
                }
            }
        }
    }
    let mut rows: Vec<Miss> = misses.into_values().collect();
    rows.sort_by_key(|m| std::cmp::Reverse(m.count));

    let total: usize = rows.iter().map(|m| m.count).sum();
    println!(
        "\n=== failure rollup — {total} missed verdict{} across {} criterion ===",
        if total == 1 { "" } else { "s" },
        rows.len(),
    );
    if rows.is_empty() {
        println!("  none");
        return;
    }
    for miss in &rows {
        println!("\n  [{}] {}", miss.count, miss.criterion);
        println!("    scenarios: {}", miss.scenarios.join(", "));
        if !miss.sample.is_empty() {
            println!("    e.g. {}", trunc(&miss.sample, 160));
        }
    }
}

fn print_failures(
    pkg: &EvalPackage,
    scenario: Option<&str>,
    events: Option<&str>,
    limit: usize,
    truncate: usize,
) {
    print_rollup(pkg, scenario);

    let mut shown = false;
    for report in &pkg.scenarios {
        if scenario.is_some_and(|sub| !report.meta.name.contains(sub)) {
            continue;
        }
        let failed: Vec<&RunRecord> = report
            .runs
            .iter()
            .filter(|r| !r.verdicts.iter().all(|v| v.passed))
            .collect();
        if failed.is_empty() {
            continue;
        }
        shown = true;
        let rule = "=".repeat(100);
        println!(
            "\n{rule}\n=== {} === rate {:.2} ({}/{} failed), gate {}\n{rule}",
            report.meta.name,
            report.aggregate.rate,
            failed.len(),
            report.runs.len(),
            if report.aggregate.gating_passed {
                "ok"
            } else {
                "FAIL"
            },
        );
        let cap = if limit == 0 { failed.len() } else { limit };
        for run in failed.iter().take(cap) {
            print_run(run, events, truncate);
        }
    }
    if !shown {
        println!(
            "\n(no failing runs to dump{})",
            match scenario {
                Some(sub) => format!(" matching '{sub}'"),
                None => String::new(),
            }
        );
    }
}

fn print_run(run: &RunRecord, events: Option<&str>, truncate: usize) {
    println!("\n──── run {} ────", run.index);
    for v in &run.verdicts {
        println!(
            "  [{}] {}",
            if v.passed { "PASS" } else { "FAIL" },
            v.criterion,
        );
        if !v.passed && !v.rationale.trim().is_empty() {
            println!("         ↳ {}", trunc(&v.rationale, truncate));
        }
    }
    if let Some(substr) = events {
        print_events(run, substr, truncate);
    }
    println!("  ── deliberation ──");
    for event in &run.events {
        match &event.payload {
            EventPayload::ConversationTurn { role, text, .. } => {
                println!("\n  «{role:?}» {}", trunc(text, truncate));
            }
            EventPayload::ModelCalled {
                phase,
                reasoning: Some(reasoning),
                ..
            } if !reasoning.trim().is_empty() => {
                println!("    · reasoning [{phase:?}]:");
                for line in trunc(reasoning, truncate).lines() {
                    println!("        {line}");
                }
            }
            EventPayload::LuaExecuted { script, result, .. } if !script.trim().is_empty() => {
                println!("      lua:");
                for line in trunc(script, truncate).lines() {
                    println!("        {line}");
                }
                if let Some(result) = result {
                    println!("      → {}", trunc(result, truncate));
                }
            }
            _ => {}
        }
    }
}

/// The events in `run` whose payload type contains `substr`, each as a one-line field summary — the
/// per-run diagnostic for pinpointing why a run failed at the event level (a wake-up that never fired,
/// an `occurred_at` that landed null or malformed, a description regenerated). Empty when none match.
fn print_events(run: &RunRecord, substr: &str, truncate: usize) {
    let lines: Vec<String> = run
        .events
        .iter()
        .filter_map(|event| summarize_event(&event.payload, substr))
        .map(|line| trunc(&line, truncate))
        .collect();
    if lines.is_empty() {
        return;
    }
    println!("  ── events [{substr}] ──");
    for line in lines {
        println!("    {line}");
    }
}

/// A compact one-line summary of a diagnostic event whose type name contains `substr`, or `None`. Only
/// the variants that carry signal for root-causing are rendered; the rest are filtered out so the lens
/// stays narrow. The `occurred_at` is rendered as its stored JSON (`{"recurring": ...}`, `{"day": ...}`,
/// or `null`) — exactly the shape that arms or fails to arm a wake-up.
fn summarize_event(payload: &EventPayload, substr: &str) -> Option<String> {
    let matches = |ty: &str| ty.contains(substr);
    match payload {
        EventPayload::MemoryContentAppended {
            id,
            occurred_at,
            text,
            visibility,
            ..
        } if matches("MemoryContentAppended") => Some(format!(
            "append {id:?} occurred_at={} visibility={visibility:?} text={:?}",
            occurred_at
                .as_ref()
                .map(|t| serde_json::to_string(t).unwrap_or_default())
                .unwrap_or_else(|| "null".to_owned()),
            text.trim(),
        )),
        EventPayload::EntryTemporalResolved {
            entry_id,
            occurred_at,
            ..
        } if matches("EntryTemporalResolved") => Some(format!(
            "resolved {entry_id:?} occurred_at={}",
            serde_json::to_string(occurred_at).unwrap_or_default(),
        )),
        EventPayload::ScheduledJobFired {
            memory, fired_at, ..
        } if matches("ScheduledJobFired") => Some(format!("fired {memory:?} @ {fired_at:?}")),
        EventPayload::ScheduledItemSurfaced {
            memory, session, ..
        } if matches("ScheduledItemSurfaced") => {
            Some(format!("surfaced {memory:?} in {session:?}"))
        }
        EventPayload::MemorySuperseded {
            entry,
            superseded_by,
            ..
        } if matches("MemorySuperseded") => {
            Some(format!("superseded {entry:?} by {superseded_by:?}"))
        }
        EventPayload::MemoryDescriptionRegenerated { id, new_text, .. }
            if matches("MemoryDescriptionRegenerated") =>
        {
            Some(format!("described {id:?}: {:?}", new_text.trim()))
        }
        _ => None,
    }
}

/// How many namespace shapes to show per relation before collapsing the tail into a `+N more` note —
/// enough to read the dominant shapes at a glance without letting a scattered relation run off the line.
const MAX_SHAPES: usize = 6;

/// The relation-vocabulary view: which typed edges the runs drew, whether each was seeded at genesis or
/// coined after, and the namespace shapes each links. This is the promotion of a projection three
/// separate sweeps hand-rolled — build an id→name map per run, then read every `LinkCreated` as
/// `from_namespace → to_namespace` cross-tabulated by relation — the canonical example the analysis skill
/// names as earning a place in `analyze`. `scenario` restricts the scan to scenarios whose name contains
/// the substring.
fn print_relations(pkg: &EvalPackage, scenario: Option<&str>) {
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
fn project_relations(pkg: &EvalPackage, scenario: Option<&str>) -> RelationsReport {
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
struct RelationsReport {
    runs_scanned: usize,
    vocab: Vec<VocabRow>,
    coinages: Vec<Coinage>,
}

/// One relation's row in the vocabulary table.
struct VocabRow {
    relation: String,
    seeded: bool,
    uses: usize,
    /// The namespace shapes this relation linked, e.g. `("person→person", 37)`, most-frequent first.
    shapes: Vec<(String, usize)>,
}

/// One relation registered outside genesis — the coinage signal.
struct Coinage {
    relation: String,
    inverse: String,
    coined_in_runs: usize,
    uses: usize,
    /// The `(scenario, run index)` pairs the relation was coined in, sorted and deduplicated.
    locations: Vec<(String, u32)>,
}

/// A coined relation accumulating its inverse and the runs it appeared in, before deduplication.
struct CoinedAcc {
    inverse: String,
    locations: Vec<(String, u32)>,
}

/// The namespace of a memory endpoint: the reserved `self` handle stands alone, an unresolvable id (no
/// `MemoryCreated` in the run) renders as a short id stub, and any other handle's namespace is whatever
/// precedes its first `/` (`person/marcus` → `person`, `context/discord:room` → `context`).
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
fn render_shapes(shapes: &[(String, usize)]) -> String {
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

/// Render coinage locations grouped by scenario with the run indices under each, e.g.
/// `infers_link_from_content #0, #2; other_scenario #1`.
fn render_locations(locations: &[(String, u32)]) -> String {
    let mut by_scenario: BTreeMap<&str, Vec<u32>> = BTreeMap::new();
    for (scenario, run) in locations {
        by_scenario.entry(scenario.as_str()).or_default().push(*run);
    }
    by_scenario
        .into_iter()
        .map(|(scenario, runs)| {
            let indices: Vec<String> = runs.iter().map(|run| format!("#{run}")).collect();
            format!("{scenario} {}", indices.join(", "))
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// The plural suffix for a count: empty for one, `s` otherwise.
fn plural(count: usize) -> &'static str {
    if count == 1 { "" } else { "s" }
}

/// Clip to `limit` characters (counting chars, not bytes, so it never splits one), noting how many
/// were dropped. `limit == 0` means no clipping — the full text.
fn trunc(text: &str, limit: usize) -> String {
    let text = text.trim();
    if limit == 0 || text.chars().count() <= limit {
        return text.to_owned();
    }
    let kept: String = text.chars().take(limit).collect();
    let dropped = text.chars().count() - limit;
    format!("{kept}… [+{dropped} chars]")
}

fn join_or_none(names: &[&str]) -> String {
    if names.is_empty() {
        "none".to_owned()
    } else {
        names.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use zuihitsu::{
        Cardinality, Event, EventPayload, LinkSource, MemoryId, MemoryName, RelationName, Seq,
        Timestamp,
    };

    use super::{project_relations, render_locations, render_shapes};
    use crate::package::{
        Aggregate, Bar, Category, EvalPackage, RunMeta, RunMetrics, RunRecord, ScenarioMeta,
        ScenarioReport, Stat, TokenStat,
    };

    fn event(payload: EventPayload) -> Event {
        Event {
            seq: Seq::ZERO,
            recorded_at: Timestamp::from_millis(0),
            payload,
        }
    }

    fn registered(name: &str, inverse: &str) -> EventPayload {
        EventPayload::LinkTypeRegistered {
            name: RelationName::new(name),
            inverse: RelationName::new(inverse),
            from_card: Cardinality::Many,
            to_card: Cardinality::Many,
            symmetric: false,
            reflexive: false,
            description: String::new(),
        }
    }

    fn created(id: MemoryId, name: &str) -> EventPayload {
        EventPayload::memory_created(id, MemoryName::new(name))
    }

    fn linked(from: MemoryId, to: MemoryId, relation: &str) -> EventPayload {
        EventPayload::LinkCreated {
            from,
            to,
            relation: RelationName::new(relation),
            source: LinkSource::Inferred,
            told_by: None,
        }
    }

    fn genesis() -> EventPayload {
        EventPayload::genesis_completed("hash", Default::default())
    }

    /// A one-run scenario carrying `events` verbatim — the projection reads only the name, run index,
    /// and events, so the aggregate is filler.
    fn scenario(name: &str, run_events: Vec<Event>) -> ScenarioReport {
        let stat = Stat {
            p50: 0.0,
            p95: 0.0,
            mean: 0.0,
        };
        ScenarioReport {
            meta: ScenarioMeta {
                name: name.to_owned(),
                category: Category::Relations,
                description: "synthetic".to_owned(),
                bar: Bar::Gating,
            },
            runs: vec![RunRecord {
                index: 0,
                started_at_ms: 0,
                finished_at_ms: 0,
                events: run_events,
                verdicts: Vec::new(),
                metrics: RunMetrics::default(),
            }],
            aggregate: Aggregate {
                runs: 1,
                rate: 1.0,
                gating_passed: true,
                wall_clock_ms: stat,
                latency_ms: stat,
                tokens: TokenStat {
                    prompt_mean: 0.0,
                    completion_mean: 0.0,
                    total_mean: 0.0,
                },
                steps_mean: 0.0,
            },
        }
    }

    fn package(scenarios: Vec<ScenarioReport>) -> EvalPackage {
        EvalPackage {
            meta: RunMeta {
                harness_version: "test".to_owned(),
                git_sha: None,
                git_dirty: false,
                model_id: "test-model".to_owned(),
                embedding_model: None,
                scenario_filter: None,
                started_at_ms: 0,
                finished_at_ms: 0,
                runs_per_scenario: 1,
                concurrency: 1,
            },
            scenarios,
        }
    }

    /// A run that seeds `knows` at genesis, coins `mentored_by` after, and draws a link under each — the
    /// canonical shape the projection tabulates.
    fn seeded_and_coined_run() -> Vec<Event> {
        let marcus = MemoryId::generate();
        let clara = MemoryId::generate();
        let zephyr = MemoryId::generate();
        vec![
            // Genesis: the seed relation is registered before the completion marker.
            event(registered("knows", "known_by")),
            event(genesis()),
            // The turn mints memories, coins a relation, and links under both.
            event(created(marcus, "person/marcus")),
            event(created(clara, "person/clara")),
            event(created(zephyr, "topic/zephyr")),
            event(linked(marcus, clara, "knows")),
            event(registered("mentored_by", "mentored")),
            event(linked(zephyr, clara, "mentored_by")),
        ]
    }

    #[test]
    fn projection_splits_seeded_from_coined_and_counts_shapes() {
        let pkg = package(vec![scenario("infers_link", seeded_and_coined_run())]);
        let report = project_relations(&pkg, None);

        assert_eq!(report.runs_scanned, 1);
        // Two relations used, both drawn once, most-used ties broken by name (mentored_by before knows).
        assert_eq!(report.vocab.len(), 2);

        let knows = report
            .vocab
            .iter()
            .find(|row| row.relation == "knows")
            .expect("knows is in the vocabulary");
        assert!(knows.seeded, "knows was registered before GenesisCompleted");
        assert_eq!(knows.uses, 1);
        assert_eq!(knows.shapes, vec![("person→person".to_owned(), 1)]);

        let mentored = report
            .vocab
            .iter()
            .find(|row| row.relation == "mentored_by")
            .expect("mentored_by is in the vocabulary");
        assert!(
            !mentored.seeded,
            "mentored_by was registered after GenesisCompleted"
        );
        assert_eq!(mentored.uses, 1);
        assert_eq!(mentored.shapes, vec![("topic→person".to_owned(), 1)]);

        // The coinage section holds exactly the post-genesis registration, with its inverse and location.
        assert_eq!(report.coinages.len(), 1);
        let coinage = &report.coinages[0];
        assert_eq!(coinage.relation, "mentored_by");
        assert_eq!(coinage.inverse, "mentored");
        assert_eq!(coinage.uses, 1);
        assert_eq!(coinage.coined_in_runs, 1);
        assert_eq!(coinage.locations, vec![("infers_link".to_owned(), 0)]);
    }

    #[test]
    fn shapes_accumulate_across_links_and_runs_sorted_by_frequency() {
        let build = || {
            let a = MemoryId::generate();
            let b = MemoryId::generate();
            let c = MemoryId::generate();
            vec![
                event(registered("knows", "known_by")),
                event(genesis()),
                event(created(a, "person/a")),
                event(created(b, "person/b")),
                event(created(c, "event/c")),
                event(linked(a, b, "knows")),
                event(linked(b, a, "knows")),
                event(linked(a, c, "knows")),
            ]
        };
        let pkg = package(vec![
            scenario("scenario_one", build()),
            scenario("scenario_two", build()),
        ]);
        let report = project_relations(&pkg, None);

        assert_eq!(report.runs_scanned, 2);
        let knows = &report.vocab[0];
        assert_eq!(knows.relation, "knows");
        assert_eq!(knows.uses, 6);
        // person→person twice per run (4 total) outranks person→event (2 total).
        assert_eq!(
            knows.shapes,
            vec![
                ("person→person".to_owned(), 4),
                ("person→event".to_owned(), 2),
            ]
        );
    }

    #[test]
    fn the_scenario_filter_narrows_the_scan() {
        let pkg = package(vec![
            scenario("merges_two_stubs", seeded_and_coined_run()),
            scenario("recalls_a_fact", seeded_and_coined_run()),
        ]);
        let report = project_relations(&pkg, Some("merges"));
        assert_eq!(report.runs_scanned, 1);
        assert_eq!(
            report.coinages[0].locations,
            vec![("merges_two_stubs".to_owned(), 0)]
        );
    }

    #[test]
    fn an_unresolvable_endpoint_renders_as_an_id_stub() {
        let ghost = MemoryId::generate();
        let marcus = MemoryId::generate();
        // No `MemoryCreated` for `ghost`, so it cannot resolve to a name.
        let events = vec![
            event(genesis()),
            event(created(marcus, "person/marcus")),
            event(linked(ghost, marcus, "knows")),
        ];
        let pkg = package(vec![scenario("orphan_link", events)]);
        let report = project_relations(&pkg, None);
        let stub: String = ghost.0.to_string().chars().take(8).collect();
        assert_eq!(report.vocab[0].shapes, vec![(format!("{stub}→person"), 1)]);
    }

    #[test]
    fn render_shapes_caps_and_notes_the_remainder() {
        let shapes: Vec<(String, usize)> = (0..8)
            .map(|index| (format!("ns{index}→person"), 8 - index))
            .collect();
        let rendered = render_shapes(&shapes);
        assert!(rendered.contains("ns0→person ×8"));
        assert!(rendered.ends_with("+2 more"));
    }

    #[test]
    fn render_locations_groups_run_indices_under_each_scenario() {
        let locations = vec![
            ("beta".to_owned(), 1),
            ("alpha".to_owned(), 2),
            ("alpha".to_owned(), 0),
        ];
        assert_eq!(render_locations(&locations), "alpha #2, #0; beta #1");
    }
}
