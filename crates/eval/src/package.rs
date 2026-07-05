//! The eval-package schema — the JSON contract between the harness and the viewer (spec §Validation →
//! the reply lane emits an eval package). A package's payload, per run, is the run's **actual event
//! log** (`Vec<Event>`), so it is a special case of the console's input; the harness adds only the
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
    /// Whether the working tree had uncommitted changes when the run started. Best-effort like
    /// `git_sha`: an unavailable or failing git reads as clean. Added additively — an older package
    /// without the field deserializes as `false`.
    #[serde(default)]
    pub git_dirty: bool,
    pub model_id: String,
    pub embedding_model: Option<String>,
    /// The `--scenario` filter the run was targeted with, verbatim; absent for a full-suite run. Added
    /// additively — an older package without the field deserializes as `None`.
    #[serde(default)]
    pub scenario_filter: Option<String>,
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
/// must-not-surface safety oracle — one regression across N fails the harness. A `RateGate` bar fails
/// the harness when the aggregate rate falls below its threshold rather than on a single slip — for
/// model-judgment behaviors with a known error band, where an occasional miss is expected but a
/// systematic slide must still fail the run. A `Metric` bar is a should-mark/should-surface rate that
/// is reported against `threshold` but never fails the run.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Bar {
    Gating,
    RateGate { threshold: f64 },
    Metric { threshold: f64 },
}

impl Bar {
    /// The bar as judged, rendered for the trend record: `gating`, `gate>=<threshold>` for a rate
    /// gate, or `>=<threshold>` for a metric bar (e.g. `>=0.6`). The archive keeps the bar each
    /// scenario was measured against so a later reader can tell a held gate from a met rate without
    /// the package.
    pub fn label(&self) -> String {
        match self {
            Bar::Gating => "gating".to_owned(),
            Bar::RateGate { threshold } => format!("gate>={threshold}"),
            Bar::Metric { threshold } => format!(">={threshold}"),
        }
    }

    /// Whether a scenario's aggregate holds this bar for the harness's exit signal: a `Gating` bar
    /// needs every gating verdict held, a `RateGate` needs the rate at or above its threshold, and a
    /// `Metric` bar never fails the run.
    pub fn holds(&self, rate: f64, gating_passed: bool) -> bool {
        match self {
            Bar::Gating => gating_passed,
            Bar::RateGate { threshold } => rate >= *threshold,
            Bar::Metric { .. } => true,
        }
    }
}

/// One run: the run's whole event log (the deliberation and resulting state the viewer reconstructs),
/// its verdicts, and the metrics computed from its `ModelCalled` events.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct RunRecord {
    pub index: u32,
    /// The harness's wall-clock (epoch milliseconds) when the run began driving and when it finished.
    /// The real clock, not the scenario's simulated one — for the viewer's live elapsed and projection.
    /// `#[serde(default)]` fills `0` so a pre-timing sidecar or package still deserializes; a `0` reads
    /// as "unstamped" and the viewer omits the per-run times rather than rendering an epoch.
    #[serde(default)]
    #[ts(type = "number")]
    pub started_at_ms: i64,
    #[serde(default)]
    #[ts(type = "number")]
    pub finished_at_ms: i64,
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
    /// A gating safety oracle's outcome — a must-not-surface property whose regression fails the
    /// harness. `judge_raw` carries the matcher's verbatim reasoning when one was consulted.
    pub fn oracle(
        criterion: impl Into<String>,
        passed: bool,
        rationale: impl Into<String>,
        judge_raw: Option<String>,
    ) -> Verdict {
        Verdict {
            criterion: criterion.into(),
            kind: VerdictKind::Oracle,
            passed,
            rationale: rationale.into(),
            judge_raw,
        }
    }

    /// A gating oracle whose rationale reads differently when it holds than when it does not — the
    /// `oracle` counterpart of [`Verdict::metric_outcome`], for a deterministic correctness property
    /// reliable enough that a regression should fail the harness rather than only lower a rate. The
    /// `when_failed` message is what makes that failure legible.
    pub fn oracle_outcome(
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
        Verdict::oracle(criterion, passed, rationale, None)
    }

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

    /// A judged quality metric, carrying the matcher's verbatim reasoning (`judge_raw`) so a rate built
    /// from model judgments stays reviewable.
    pub fn metric_judged(
        criterion: impl Into<String>,
        passed: bool,
        rationale: impl Into<String>,
        judge_raw: String,
    ) -> Verdict {
        Verdict {
            criterion: criterion.into(),
            kind: VerdictKind::Metric,
            passed,
            rationale: rationale.into(),
            judge_raw: Some(judge_raw),
        }
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
    /// The run's actual wall-clock to drive (turns plus the synchronous catch-ups it forces). The
    /// truthful cost: unlike `total_latency_ms`, it includes work that records no `ModelCalled` — the
    /// background describer's synthesis, run synchronously in the harness.
    #[ts(type = "number")]
    pub wall_clock_ms: u64,
    /// Summed `ModelCalled` duration. Conversational model calls only — off-hot-path synthesis records
    /// no `ModelCalled`, so this undercounts total compute; read `wall_clock_ms` for that.
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
    /// The per-run drive wall-clock distribution (the truthful cost; see [`RunMetrics::wall_clock_ms`]).
    pub wall_clock_ms: Stat,
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

#[cfg(test)]
mod tests {
    use super::RunRecord;

    /// A pre-timing package predates the wall-clock stamps; `#[serde(default)]` must fill `0` so the
    /// old record still deserializes, and a `0` reads as "unstamped" for the viewer.
    #[test]
    fn a_run_record_without_stamps_defaults_them_to_zero() {
        let old = r#"{"index":3,"events":[],"verdicts":[],"metrics":{"model_calls":0,"steps":0,"wall_clock_ms":0,"total_latency_ms":0,"prompt_tokens":0,"completion_tokens":0,"total_tokens":0,"gating_passed":true}}"#;
        let record: RunRecord = serde_json::from_str(old).expect("old-shape record parses");
        assert_eq!(record.index, 3);
        assert_eq!(record.started_at_ms, 0);
        assert_eq!(record.finished_at_ms, 0);
    }

    /// A stamped record round-trips through JSON, carrying both wall-clock stamps.
    #[test]
    fn a_stamped_run_record_round_trips() {
        let record = RunRecord {
            index: 1,
            started_at_ms: 1_700_000_000_000,
            finished_at_ms: 1_700_000_042_000,
            events: Vec::new(),
            verdicts: Vec::new(),
            metrics: super::RunMetrics::default(),
        };
        let json = serde_json::to_string(&record).expect("serializes");
        let back: RunRecord = serde_json::from_str(&json).expect("round-trips");
        assert_eq!(back.started_at_ms, 1_700_000_000_000);
        assert_eq!(back.finished_at_ms, 1_700_000_042_000);
    }
}
