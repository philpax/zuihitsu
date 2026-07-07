//! The v2 trend record: one compact, deterministically-ordered line per run, appended to the tracked
//! history (spec §Validation → the tracked metrics trend).

use std::path::Path;

use serde::Serialize;

use crate::{
    error::EvalError,
    harness,
    package::{EvalPackage, ScenarioReport, VerdictKind},
};

/// The v2 trend record: one compact, deterministically-ordered line per run, appended to the tracked
/// history (spec §Validation → the tracked metrics trend). Carries the run's `name` so a record
/// correlates back to its `eval/<name>.json` package, real wall-clock stamps, the git state it ran at,
/// and, per scenario, the bar it was judged against and the per-criterion pass tallies for aggregate
/// analysis.
#[derive(Serialize)]
pub(crate) struct HistoryLine {
    name: String,
    /// Epoch milliseconds — the real wall-clock span (`ts_ms` is retired in favor of these).
    started_at_ms: i64,
    finished_at_ms: i64,
    /// The commit the run ran at, or the empty string when git could not resolve one (best-effort).
    git_sha: String,
    /// Whether the working tree had uncommitted changes when the run started.
    git_dirty: bool,
    model_id: String,
    runs_per_scenario: u32,
    /// The `--scenario` filter the run was targeted with; omitted for a full-suite run.
    #[serde(skip_serializing_if = "Option::is_none")]
    scenario_filter: Option<String>,
    scenarios: Vec<HistoryScenario>,
}

#[derive(Serialize)]
pub(crate) struct HistoryScenario {
    name: String,
    rate: f64,
    gating_passed: bool,
    /// Runs actually completed for this scenario — resume can make this differ from `runs_per_scenario`.
    runs: u32,
    /// The bar this scenario was judged against, rendered (e.g. `gating` or `>=0.6`).
    bar: String,
    wall_clock_p50_ms: u64,
    latency_p50_ms: u64,
    /// The median per-run step count.
    steps_p50: f64,
    total_tokens_mean: u64,
    /// Per-criterion pass tallies aggregated across the scenario's runs.
    criteria: Vec<CriterionStat>,
}

/// One criterion's pass tally across a scenario's runs: how many of the `total` runs that judged it
/// passed. `kind` distinguishes a gating oracle from a reported metric.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct CriterionStat {
    pub(crate) criterion: String,
    pub(crate) kind: String,
    pub(crate) passed: u32,
    pub(crate) total: u32,
}

/// Build the v2 history line for a completed run.
pub(crate) fn history_line(name: &str, package: &EvalPackage) -> HistoryLine {
    HistoryLine {
        name: name.to_owned(),
        started_at_ms: package.meta.started_at_ms,
        finished_at_ms: package.meta.finished_at_ms,
        git_sha: package.meta.git_sha.clone().unwrap_or_default(),
        git_dirty: package.meta.git_dirty,
        model_id: package.meta.model_id.clone(),
        runs_per_scenario: package.meta.runs_per_scenario,
        scenario_filter: package.meta.scenario_filter.clone(),
        scenarios: package
            .scenarios
            .iter()
            .map(|report| {
                let steps: Vec<f64> = report
                    .runs
                    .iter()
                    .map(|run| run.metrics.steps as f64)
                    .collect();
                HistoryScenario {
                    name: report.meta.name.clone(),
                    // Round so an unchanged result produces an identical line (clean diffs/appends).
                    rate: (report.aggregate.rate * 1000.0).round() / 1000.0,
                    gating_passed: report.aggregate.gating_passed,
                    runs: report.aggregate.runs,
                    bar: report.meta.bar.label(),
                    wall_clock_p50_ms: report.aggregate.wall_clock_ms.p50.round() as u64,
                    latency_p50_ms: report.aggregate.latency_ms.p50.round() as u64,
                    steps_p50: harness::percentile(&steps, 0.50),
                    total_tokens_mean: report.aggregate.tokens.total_mean.round() as u64,
                    criteria: criteria_stats(report),
                }
            })
            .collect(),
    }
}

/// Aggregate the per-criterion pass tallies across a scenario's runs, keyed by `(criterion, kind)` and
/// ordered deterministically (by criterion, then kind) so an unchanged result produces an identical
/// line. A criterion's `total` counts the runs that judged it, and `passed` those where it held.
pub(crate) fn criteria_stats(report: &ScenarioReport) -> Vec<CriterionStat> {
    use std::collections::BTreeMap;

    let mut tallies: BTreeMap<(String, &'static str), (u32, u32)> = BTreeMap::new();
    for run in &report.runs {
        for verdict in &run.verdicts {
            let kind = match verdict.kind {
                VerdictKind::Oracle => "oracle",
                VerdictKind::Metric => "metric",
            };
            let entry = tallies
                .entry((verdict.criterion.clone(), kind))
                .or_default();
            entry.1 += 1;
            if verdict.passed {
                entry.0 += 1;
            }
        }
    }
    tallies
        .into_iter()
        .map(|((criterion, kind), (passed, total))| CriterionStat {
            criterion,
            kind: kind.to_owned(),
            passed,
            total,
        })
        .collect()
}

pub(crate) fn append_history(name: &str, package: &EvalPackage) -> Result<(), EvalError> {
    use std::io::Write as _;

    let line = history_line(name, package);
    let path = Path::new("eval/history.jsonl");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| EvalError::WriteOutput {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|source| EvalError::WriteOutput {
            path: path.to_path_buf(),
            source,
        })?;
    let mut json = serde_json::to_string(&line)?;
    json.push('\n');
    file.write_all(json.as_bytes())
        .map_err(|source| EvalError::WriteOutput {
            path: path.to_path_buf(),
            source,
        })?;
    Ok(())
}
