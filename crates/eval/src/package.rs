//! The eval-package schema — the JSON contract between the harness and the viewer (spec §Validation →
//! the reply lane emits an eval package). A package's payload, per run, is the run's **actual event
//! log** (`Vec<Event>`), so it is a special case of the console's input; the harness adds only the
//! per-run verdicts and the computed metrics. The types are defined in `zuihitsu-frontend-types`
//! and re-exported here so the existing `crate::package::*` paths keep working.

pub use zuihitsu_frontend_types::{
    Aggregate, Bar, Category, EvalPackage, PackageSummary, ResumeProvenance, RunMeta, RunMetrics,
    RunRecord, RunSummary, ScenarioMeta, ScenarioReport, Stat, TokenStat, Verdict, VerdictKind,
};

use crate::{error::EvalError, judge::JudgeOutcome};

/// A judged verdict, from the judge's outcome for `criterion`. A judge error is not a harness
/// crash: it becomes a failed verdict carrying the error, so a flaky judge call lowers the rate
/// rather than aborting the run.
///
/// This is a free function rather than an associated function on [`Verdict`] because `Verdict` is
/// defined in `zuihitsu-frontend-types`, which does not depend on the eval crate — so the eval-only
/// types (`JudgeOutcome`, `EvalError`) cannot appear in a method on `Verdict` there, and Rust does
/// not allow inherent `impl` blocks for a type outside the crate where it is defined.
pub fn verdict_from_judge_outcome(
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
