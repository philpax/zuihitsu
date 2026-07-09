//! The `replay --mode rejudge` path: re-assess a recorded package against the current scenario oracles
//! without re-running the model, to see how an oracle or judge change reclassifies an existing eval. The
//! recorded event logs are the input; only the verdicts are recomputed. Never writes trend history.

use std::{collections::BTreeMap, path::Path, sync::Arc};

use zuihitsu::{EnvConfig, ModelClient, OpenAiClient};

use crate::{
    error::EvalError,
    harness,
    judge::Judge,
    package::{Bar, ScenarioReport, Verdict, VerdictKind},
    retry::RetryingModel,
    scenario::Scenario,
};

/// Re-assess `package`'s runs against the current registry's oracles. `scenario` restricts to
/// scenarios whose name contains the substring. With `name`, write the re-judged package to
/// `eval/<name>.json` (its runs keep their events and journal, carry the new verdicts, and its meta
/// records the source); without it, only report.
pub(crate) async fn rejudge(
    package: &Path,
    scenario: Option<&str>,
    config_path: &Path,
    name: Option<&str>,
) -> Result<(), EvalError> {
    let pkg = crate::replay::load(package)?;
    let judge = build_judge(config_path)?;
    let registry = crate::scenarios::all();

    let source_stem = package
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();

    let mut rejudged = pkg.clone();
    let mut comparisons = Vec::new();
    for (report, out) in pkg.scenarios.iter().zip(rejudged.scenarios.iter_mut()) {
        if scenario.is_some_and(|sub| !report.meta.name.contains(sub)) {
            continue;
        }
        let Some(scenario) = registry
            .iter()
            .find(|scenario| scenario.meta().name == report.meta.name)
        else {
            tracing::warn!(
                scenario = %report.meta.name,
                "not in the current registry; skipping — its recorded verdicts are kept as-is",
            );
            continue;
        };
        let comparison = rejudge_scenario(report, out, scenario.as_ref(), &judge).await;
        comparisons.push(comparison);
    }

    print_report(&comparisons);

    if let Some(name) = name {
        rejudged.meta.rejudged_from = Some(source_stem);
        let out = Path::new(crate::run::EVAL_DIR).join(format!("{name}.json"));
        crate::run::write_package(&rejudged, &out)?;
        println!("\nwrote the re-judged package to {}", out.display());
    }
    Ok(())
}

/// Build the judge from `config_path` — the model alone, the minimal path a rejudge needs (no embedder,
/// no MCP, no run deps). The judge is the model run clean-room.
fn build_judge(config_path: &Path) -> Result<Judge, EvalError> {
    let config = EnvConfig::load(config_path).map_err(|source| EvalError::LoadConfig {
        path: config_path.to_path_buf(),
        source: Box::new(source),
    })?;
    if config.model.endpoint.is_empty() {
        return Err(EvalError::Replay(
            "rejudge needs a model endpoint to re-assess; none is configured".to_owned(),
        ));
    }
    let model: Arc<dyn ModelClient> = Arc::new(RetryingModel::new(Arc::new(OpenAiClient::new(
        &config.model,
    ))));
    Ok(Judge::new(model))
}

/// Re-assess one scenario's runs, writing the fresh verdicts into `out` (its runs' metrics gating flag
/// and its aggregate recomputed) and returning the recorded-vs-rejudged comparison.
async fn rejudge_scenario(
    recorded: &ScenarioReport,
    out: &mut ScenarioReport,
    scenario: &dyn Scenario,
    judge: &Judge,
) -> ScenarioComparison {
    let mut run_verdicts = Vec::with_capacity(recorded.runs.len());
    for (recorded_run, out_run) in recorded.runs.iter().zip(out.runs.iter_mut()) {
        let fresh = scenario.assess(&recorded_run.events, judge).await;
        out_run.metrics.gating_passed = gating_passed(&fresh);
        out_run.verdicts = fresh.clone();
        run_verdicts.push(RunVerdicts {
            index: recorded_run.index,
            recorded: recorded_run.verdicts.clone(),
            rejudged: fresh,
        });
    }
    out.aggregate = harness::aggregate(&out.runs);
    compare_runs(&recorded.meta.name, recorded.meta.bar, &run_verdicts)
}

/// One run's recorded and freshly-re-judged verdict sets, paired for comparison.
pub(crate) struct RunVerdicts {
    pub(crate) index: u32,
    pub(crate) recorded: Vec<Verdict>,
    pub(crate) rejudged: Vec<Verdict>,
}

/// One criterion's recorded-vs-rejudged pass tallies across a scenario's runs.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct CriterionDelta {
    pub(crate) criterion: String,
    pub(crate) kind: VerdictKind,
    pub(crate) recorded_passed: u32,
    pub(crate) rejudged_passed: u32,
    pub(crate) total: u32,
}

/// One run×criterion cell that flipped between the recording and the re-judgment.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Flip {
    pub(crate) run: u32,
    pub(crate) criterion: String,
    /// The recorded pass state (the re-judged state is its negation).
    pub(crate) recorded_pass: bool,
    /// The re-judged rationale — what the fresh assessment said, so a pass→fail flip is legible.
    pub(crate) rejudged_rationale: String,
}

/// A scenario's recorded-vs-rejudged comparison — the report's data layer, computed purely so it is
/// tested directly.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ScenarioComparison {
    pub(crate) name: String,
    pub(crate) criteria: Vec<CriterionDelta>,
    pub(crate) flips: Vec<Flip>,
    pub(crate) recorded_bar_held: bool,
    pub(crate) rejudged_bar_held: bool,
}

/// Compare a scenario's recorded and re-judged verdicts, per run paired by criterion name. Accumulates
/// per-criterion pass tallies (ordered deterministically by criterion, then kind), collects the flipped
/// cells, and recomputes each side's bar disposition. Pure over the verdict data.
pub(crate) fn compare_runs(name: &str, bar: Bar, runs: &[RunVerdicts]) -> ScenarioComparison {
    let mut tallies: BTreeMap<(String, &'static str), CriterionDelta> = BTreeMap::new();
    let mut flips = Vec::new();

    for run in runs {
        for recorded in &run.recorded {
            let rejudged = run
                .rejudged
                .iter()
                .find(|verdict| verdict.criterion == recorded.criterion);
            let kind_key = kind_key(recorded.kind);
            let entry = tallies
                .entry((recorded.criterion.clone(), kind_key))
                .or_insert_with(|| CriterionDelta {
                    criterion: recorded.criterion.clone(),
                    kind: recorded.kind,
                    recorded_passed: 0,
                    rejudged_passed: 0,
                    total: 0,
                });
            entry.total += 1;
            if recorded.passed {
                entry.recorded_passed += 1;
            }
            if let Some(rejudged) = rejudged {
                if rejudged.passed {
                    entry.rejudged_passed += 1;
                }
                if rejudged.passed != recorded.passed {
                    flips.push(Flip {
                        run: run.index,
                        criterion: recorded.criterion.clone(),
                        recorded_pass: recorded.passed,
                        rejudged_rationale: rejudged.rationale.clone(),
                    });
                }
            }
        }
    }

    let recorded_gating = gating_stats(runs.iter().map(|run| run.recorded.as_slice()));
    let rejudged_gating = gating_stats(runs.iter().map(|run| run.rejudged.as_slice()));
    ScenarioComparison {
        name: name.to_owned(),
        criteria: tallies.into_values().collect(),
        flips,
        recorded_bar_held: bar.holds(recorded_gating.0, recorded_gating.1),
        rejudged_bar_held: bar.holds(rejudged_gating.0, rejudged_gating.1),
    }
}

/// Whether every oracle verdict in `verdicts` held (the per-run gating flag).
fn gating_passed(verdicts: &[Verdict]) -> bool {
    verdicts
        .iter()
        .filter(|verdict| matches!(verdict.kind, VerdictKind::Oracle))
        .all(|verdict| verdict.passed)
}

/// The `(gating_rate, gating_passed)` a bar is judged against, over a set of per-run verdict lists: the
/// fraction of runs whose oracle verdicts all held, and whether every run's did.
fn gating_stats<'a>(runs: impl Iterator<Item = &'a [Verdict]>) -> (f64, bool) {
    let mut total = 0u32;
    let mut held = 0u32;
    for verdicts in runs {
        total += 1;
        if gating_passed(verdicts) {
            held += 1;
        }
    }
    if total == 0 {
        return (1.0, true);
    }
    (held as f64 / total as f64, held == total)
}

fn kind_key(kind: VerdictKind) -> &'static str {
    match kind {
        VerdictKind::Oracle => "oracle",
        VerdictKind::Metric => "metric",
    }
}

/// Print the recorded-vs-rejudged comparison: per scenario, the per-criterion rates with their delta,
/// the flipped cells, and the recomputed bar disposition; then an overall summary line.
fn print_report(comparisons: &[ScenarioComparison]) {
    if comparisons.is_empty() {
        println!("\n(no scenarios re-judged — none matched, or none are in the current registry)");
        return;
    }

    let mut flipped_scenarios = 0;
    let mut bar_changes = 0;
    for comparison in comparisons {
        println!("\n=== {} ===", comparison.name);
        for criterion in &comparison.criteria {
            let recorded = rate(criterion.recorded_passed, criterion.total);
            let rejudged = rate(criterion.rejudged_passed, criterion.total);
            let delta = rejudged - recorded;
            println!(
                "  [{}] {:<48} {:.2} → {:.2}  ({:+.2})",
                kind_key(criterion.kind),
                criterion.criterion,
                recorded,
                rejudged,
                delta,
            );
        }
        if comparison.flips.is_empty() {
            println!("  no flips");
        } else {
            flipped_scenarios += 1;
            for flip in &comparison.flips {
                let direction = if flip.recorded_pass {
                    "PASS → FAIL"
                } else {
                    "FAIL → PASS"
                };
                println!("  flip run {} [{}] {}", flip.run, direction, flip.criterion);
                if flip.recorded_pass {
                    println!("       ↳ {}", flip.rejudged_rationale.trim());
                }
            }
        }
        let bar = |held: bool| if held { "held" } else { "NOT held" };
        if comparison.recorded_bar_held != comparison.rejudged_bar_held {
            bar_changes += 1;
            println!(
                "  bar: {} → {}  (CHANGED)",
                bar(comparison.recorded_bar_held),
                bar(comparison.rejudged_bar_held),
            );
        } else {
            println!("  bar: {}", bar(comparison.rejudged_bar_held));
        }
    }

    println!(
        "\n{} scenario(s) re-judged; {} with flips; {} bar disposition change(s)",
        comparisons.len(),
        flipped_scenarios,
        bar_changes,
    );
}

fn rate(passed: u32, total: u32) -> f64 {
    if total == 0 {
        0.0
    } else {
        passed as f64 / total as f64
    }
}
