//! The v2 trend record: one compact, deterministically-ordered line per run, appended to the tracked
//! history (spec §Validation → the tracked metrics trend).

use std::{collections::BTreeMap, path::Path};

use serde::{Deserialize, Serialize};

/// The tracked trend record every finished run appends to, and the only source of
/// per-scenario timing a pre-flight estimate can draw on.
const HISTORY_PATH: &str = "eval/history.jsonl";

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
#[derive(Deserialize, Serialize)]
pub(crate) struct HistoryLine {
    pub(crate) name: String,
    /// Epoch milliseconds — the real wall-clock span (`ts_ms` is retired in favor of these).
    started_at_ms: i64,
    finished_at_ms: i64,
    /// The commit the run ran at, or the empty string when git could not resolve one (best-effort).
    pub(crate) git_sha: String,
    /// Whether the working tree had uncommitted changes when the run started.
    git_dirty: bool,
    model_id: String,
    runs_per_scenario: u32,
    /// The `--scenario` filter the run was targeted with; omitted for a full-suite run.
    #[serde(skip_serializing_if = "Option::is_none")]
    scenario_filter: Option<String>,
    pub(crate) scenarios: Vec<HistoryScenario>,
}

#[derive(Deserialize, Serialize)]
pub(crate) struct HistoryScenario {
    pub(crate) name: String,
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
    pub(crate) criteria: Vec<CriterionStat>,
}

/// One criterion's pass tally across a scenario's runs: how many of the `total` runs that judged it
/// passed. `kind` distinguishes a gating oracle from a reported metric.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
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
    let path = Path::new(HISTORY_PATH);
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

/// A pre-flight projection of a run's wall-clock, summed per scenario rather than averaged across
/// them. Scenario cost is wildly uneven — a compaction scenario drives several sessions and runs
/// minutes, a synthesis one seconds — so a flat per-run average misestimates any run whose scenario
/// mix differs from the suite's, which every `--scenario` run does by construction.
pub(crate) struct Estimate {
    /// The projected wall-clock for the whole run, in milliseconds.
    pub(crate) total_ms: u64,
    /// How many scenarios fell back to the median of the timed ones, never having been timed
    /// themselves — the projection's confidence, in one number.
    pub(crate) unknown: usize,
}

/// Project how long driving `plan` will take — each entry a scenario and the number of runs still to
/// drive for it — from the per-scenario medians in the tracked history. Each scenario contributes its
/// own most recent `wall_clock_p50_ms`; one never timed before contributes the median of those that
/// have, so a new scenario does not read as free.
///
/// Taking a per-scenario run count rather than one flat `runs` is what makes a resume honest: the
/// runs left are rarely a uniform slice of the suite, and here they are typically its slowest tail,
/// so scaling a whole-suite total by the fraction remaining would under-project badly.
///
/// `None` when the history holds no timing at all — a first run on a fresh checkout has nothing to
/// project from, and a fabricated number would be worse than none.
pub(crate) fn estimate(plan: &[(String, u32)], concurrency: usize) -> Option<Estimate> {
    project(&recent_timings(), plan, concurrency)
}

/// The projection itself, over a timings map the caller supplies — split from [`estimate`] so the
/// arithmetic is exercised without a history file on disk.
fn project(
    timings: &BTreeMap<String, u64>,
    plan: &[(String, u32)],
    concurrency: usize,
) -> Option<Estimate> {
    if timings.is_empty() {
        return None;
    }
    let mut sorted: Vec<u64> = timings.values().copied().collect();
    sorted.sort_unstable();
    let median = sorted[sorted.len() / 2];

    let (mut total, mut unknown) = (0u64, 0usize);
    for (scenario, runs) in plan {
        let per_run = match timings.get(scenario) {
            Some(per_run) => *per_run,
            None => {
                unknown += 1;
                median
            }
        };
        total += per_run * u64::from(*runs);
    }
    Some(Estimate {
        // Runs in flight divide the wall-clock; concurrency defaults to 1, where this is the identity.
        total_ms: total / concurrency.max(1) as u64,
        unknown,
    })
}

/// Every tracked history line, oldest first; empty when the history is absent or unreadable. A line
/// that will not parse is skipped rather than fatal, so one malformed row cannot blind the trend.
pub(crate) fn read_all() -> Vec<HistoryLine> {
    let Ok(text) = std::fs::read_to_string(HISTORY_PATH) else {
        return Vec::new();
    };
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

/// Each scenario's most recently recorded per-run median, by name. Later lines win, so a scenario
/// whose cost changed reads at its newest measurement rather than an average over its whole history.
fn recent_timings() -> BTreeMap<String, u64> {
    let mut timings = BTreeMap::new();
    let Ok(text) = std::fs::read_to_string(HISTORY_PATH) else {
        return timings;
    };
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let Ok(record) = serde_json::from_str::<HistoryLine>(line) else {
            continue;
        };
        for scenario in record.scenarios {
            timings.insert(scenario.name, scenario.wall_clock_p50_ms);
        }
    }
    timings
}

#[cfg(test)]
mod estimate_tests {
    use super::*;

    fn timings() -> BTreeMap<String, u64> {
        BTreeMap::from([
            ("fast".to_owned(), 10_000),
            ("middling".to_owned(), 40_000),
            ("slow".to_owned(), 160_000),
        ])
    }

    #[test]
    fn a_scenario_contributes_its_own_measured_cost() {
        // The point of the whole projection: scenario cost spans more than an order of magnitude, so
        // a plan weighted toward the slow ones must project longer than the same run count of fast
        // ones, where a flat per-run mean would give both the same answer.
        let slow = project(&timings(), &[("slow".to_owned(), 5)], 1).unwrap();
        let fast = project(&timings(), &[("fast".to_owned(), 5)], 1).unwrap();
        assert_eq!(slow.total_ms, 800_000);
        assert_eq!(fast.total_ms, 50_000);
    }

    #[test]
    fn a_resume_projects_only_the_runs_left_for_each_scenario() {
        // The tail of an interrupted suite is not a uniform slice of it — here the slow scenario is
        // all that remains, so the projection must follow the plan, not the suite's shape.
        let plan = vec![("fast".to_owned(), 0), ("slow".to_owned(), 3)];
        assert_eq!(project(&timings(), &plan, 1).unwrap().total_ms, 480_000);
    }

    #[test]
    fn an_untimed_scenario_costs_the_median_rather_than_nothing() {
        let estimate = project(&timings(), &[("brand-new".to_owned(), 2)], 1).unwrap();
        assert_eq!(estimate.total_ms, 80_000, "the median is 40s per run");
        assert_eq!(estimate.unknown, 1);
    }

    #[test]
    fn concurrency_divides_the_wall_clock() {
        let plan = vec![("middling".to_owned(), 4)];
        assert_eq!(project(&timings(), &plan, 1).unwrap().total_ms, 160_000);
        assert_eq!(project(&timings(), &plan, 4).unwrap().total_ms, 40_000);
    }

    #[test]
    fn an_empty_history_projects_nothing_rather_than_guessing() {
        assert!(project(&BTreeMap::new(), &[("fast".to_owned(), 5)], 1).is_none());
    }
}
