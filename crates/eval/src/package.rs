//! The eval-package schema — the JSON contract between the harness and the viewer (spec §Validation →
//! the reply lane emits an eval package). A package's payload, per run, is the run's **actual event
//! log** (`Vec<Event>`), so it is a special case of the debugger's input; the harness adds only the
//! per-run verdicts and the computed metrics. These types derive `ts_rs::TS`, and `export-types` emits
//! them — plus the whole transitively-referenced event-log graph — as TypeScript for the viewer.

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use zuihitsu::event::Event;

use crate::{error::EvalError, judge::JudgeOutcome};

/// One eval run over the whole scenario suite.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct EvalPackage {
    pub meta: RunMeta,
    pub scenarios: Vec<ScenarioReport>,
}

/// What produced this package: the harness, the models, and the wall-clock span.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct RunMeta {
    pub harness_version: String,
    /// The repository commit the harness ran at, when resolvable.
    pub git_sha: Option<String>,
    pub model_id: String,
    pub embedding_model: Option<String>,
    /// Epoch milliseconds.
    #[ts(type = "number")]
    pub started_at_ms: i64,
    #[ts(type = "number")]
    pub finished_at_ms: i64,
    pub runs_per_scenario: u32,
    pub concurrency: u32,
}

/// One scenario's N runs plus their aggregate.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct ScenarioReport {
    pub meta: ScenarioMeta,
    pub runs: Vec<RunRecord>,
    pub aggregate: Aggregate,
}

/// A scenario's identity and its bar — the rubric the aggregate is read against.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct ScenarioMeta {
    pub name: String,
    pub category: Category,
    pub description: String,
    pub bar: Bar,
}

/// The scenario families. Descriptive groupings for the viewer; the set grows over time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Recall,
    Tagging,
    Relations,
    Scheduling,
    Privacy,
    Compaction,
    Arbitration,
    Description,
}

/// How a scenario's runs are judged (spec §Validation → gating versus measurement). A `Gating` bar is a
/// must-not-surface safety oracle — one regression across N fails the harness. A `Metric` bar is a
/// should-mark/should-surface rate that is reported against `threshold` but never fails the run.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Bar {
    Gating,
    Metric { threshold: f64 },
}

/// One run: the run's whole event log (the deliberation and resulting state the viewer reconstructs),
/// its verdicts, and the metrics computed from its `ModelCalled` events.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct RunRecord {
    pub index: u32,
    pub events: Vec<Event>,
    pub verdicts: Vec<Verdict>,
    pub metrics: RunMetrics,
}

/// One oracle's outcome for a run. `judge_raw` carries the judge model's verbatim response when a
/// criterion was judged (rather than checked deterministically), so the matcher stays reviewable.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct Verdict {
    pub criterion: String,
    pub kind: VerdictKind,
    pub passed: bool,
    pub rationale: String,
    pub judge_raw: Option<String>,
}

impl Verdict {
    /// A deterministically-checked quality metric (no judge; `judge_raw` is `None`).
    pub fn metric(
        criterion: impl Into<String>,
        passed: bool,
        rationale: impl Into<String>,
    ) -> Verdict {
        Verdict {
            criterion: criterion.into(),
            kind: VerdictKind::Metric,
            passed,
            rationale: rationale.into(),
            judge_raw: None,
        }
    }

    /// A deterministic metric whose rationale reads differently when it holds than when it does not —
    /// the common "passed: did X / failed: did not do X" shape. `when_passed` is recorded if `passed`,
    /// `when_failed` otherwise.
    pub fn metric_outcome(
        criterion: impl Into<String>,
        passed: bool,
        when_passed: impl Into<String>,
        when_failed: impl Into<String>,
    ) -> Verdict {
        let rationale = if passed {
            when_passed.into()
        } else {
            when_failed.into()
        };
        Verdict::metric(criterion, passed, rationale)
    }

    /// A judged verdict, from the judge's outcome for `criterion`. A judge error is not a harness
    /// crash: it becomes a failed verdict carrying the error, so a flaky judge call lowers the rate
    /// rather than aborting the run.
    pub fn from_judge_outcome(
        criterion: impl Into<String>,
        kind: VerdictKind,
        outcome: Result<JudgeOutcome, EvalError>,
    ) -> Verdict {
        let criterion = criterion.into();
        match outcome {
            Ok(outcome) => Verdict {
                criterion,
                kind,
                passed: outcome.passed,
                rationale: outcome.rationale,
                judge_raw: Some(outcome.raw),
            },
            Err(error) => Verdict {
                criterion,
                kind,
                passed: false,
                rationale: format!("judge error: {error}"),
                judge_raw: None,
            },
        }
    }
}

/// Whether a verdict is a gating safety oracle or a reported quality metric.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum VerdictKind {
    Oracle,
    Metric,
}

/// Per-run measurements, summed from the run's `ModelCalled` events.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, TS)]
pub struct RunMetrics {
    pub model_calls: u32,
    pub steps: u32,
    #[ts(type = "number")]
    pub total_latency_ms: u64,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
    /// Whether every gating oracle in this run passed.
    pub gating_passed: bool,
}

/// A scenario's aggregate across its N runs.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, TS)]
pub struct Aggregate {
    pub runs: u32,
    /// The pass rate over the runs (1.0 = every run passed every oracle of its bar).
    pub rate: f64,
    /// True iff every gating oracle held in every run (the safety invariant; drives the exit code).
    pub gating_passed: bool,
    pub latency_ms: Stat,
    pub tokens: TokenStat,
    pub steps_mean: f64,
}

/// A latency distribution across runs (milliseconds).
#[derive(Clone, Copy, Debug, Serialize, Deserialize, TS)]
pub struct Stat {
    pub p50: f64,
    pub p95: f64,
    pub mean: f64,
}

/// Mean token usage across runs.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, TS)]
pub struct TokenStat {
    pub prompt_mean: f64,
    pub completion_mean: f64,
    pub total_mean: f64,
}
