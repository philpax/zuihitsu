//! `eval events` and `eval replay` — the tools for reading a recorded run's events and replaying a
//! recorded package. `events` groups a run's events under the journal steps that produced them, so an
//! operator can pick a `--step` to resume from. `replay` has two modes: `rejudge` re-assesses a recorded
//! package against the current oracles without re-running the model, and `resume` rewinds one run to a
//! chosen step and redoes the rest of the scenario live. Neither ever writes trend history.
//!
//! Not to be confused with `run --resume`, which continues an interrupted *suite* run from its `.jsonl`
//! sidecar (`live::resume`); that recovers a partial run, whereas `replay --mode resume` re-drives a
//! completed one from a mid-scenario step.

mod events;
mod rejudge;
mod render;
mod resume;

#[cfg(test)]
mod tests;

use std::path::Path;

use clap::ValueEnum;

use crate::{
    analyze::load,
    error::EvalError,
    package::{EvalPackage, RunRecord, ScenarioReport},
};

pub(crate) use events::events;

/// Which replay mode to run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum ReplayMode {
    /// Re-assess the recorded runs against the current criteria without re-running the model — for
    /// testing how an oracle or judge change reclassifies an existing eval.
    Rejudge,
    /// Rewind one run to `--step` and redo the rest of the scenario live from that point with the
    /// current code and model.
    Resume,
}

/// The arguments a `replay` invocation carries, mapped straight from the CLI subcommand's flags. The two
/// modes share the package and config but diverge on the rest — `resume` needs a scenario, run, and
/// step; `rejudge` rejects them — so the mode-specific fields are optional and validated per mode.
pub(crate) struct ReplayRequest<'a> {
    pub(crate) package: &'a Path,
    pub(crate) mode: ReplayMode,
    pub(crate) scenario: Option<&'a str>,
    pub(crate) run: Option<usize>,
    pub(crate) step: Option<u32>,
    pub(crate) config: &'a Path,
    pub(crate) name: Option<&'a str>,
}

/// Dispatch a `replay` invocation, validating the mode-specific arguments first (before touching the
/// config or the model), then running the chosen mode.
pub(crate) async fn replay(request: ReplayRequest<'_>) -> Result<(), EvalError> {
    let ReplayRequest {
        package,
        mode,
        scenario,
        run,
        step,
        config,
        name,
    } = request;
    match mode {
        ReplayMode::Rejudge => {
            if run.is_some() || step.is_some() {
                return Err(EvalError::Replay(
                    "rejudge re-assesses every recorded run; it takes neither --run nor --step \
                     (use --scenario to restrict)"
                        .to_owned(),
                ));
            }
            rejudge::rejudge(package, scenario, config, name).await
        }
        ReplayMode::Resume => {
            let scenario = scenario.ok_or_else(|| {
                EvalError::Replay(
                    "resume requires --scenario to select exactly one scenario to resume"
                        .to_owned(),
                )
            })?;
            let run = run.ok_or_else(|| {
                EvalError::Replay("resume requires --run to select the run to resume".to_owned())
            })?;
            let step = step.ok_or_else(|| {
                EvalError::Replay(
                    "resume requires --step to select the last journal step to keep".to_owned(),
                )
            })?;
            resume::resume(package, scenario, run, step, config, name).await
        }
    }
}

/// Resolve the target scenario in `pkg`: with a `filter`, the scenarios whose name contains it;
/// without one, all of them. Exactly one candidate resolves; zero or several is an error naming the
/// scenarios so the caller can narrow with `--scenario`. Pure over the package, so it is tested
/// directly. Returns a plain message; the caller wraps it in the mode's error context.
pub(crate) fn resolve_scenario<'a>(
    pkg: &'a EvalPackage,
    filter: Option<&str>,
) -> Result<&'a ScenarioReport, String> {
    let candidates: Vec<&ScenarioReport> = pkg
        .scenarios
        .iter()
        .filter(|report| filter.is_none_or(|sub| report.meta.name.contains(sub)))
        .collect();
    match candidates.as_slice() {
        [only] => Ok(only),
        [] => Err(match filter {
            Some(sub) => format!(
                "no scenario matches {sub:?}; the package holds: {}",
                names(&pkg.scenarios),
            ),
            None => "the package holds no scenarios".to_owned(),
        }),
        several => Err(format!(
            "{} scenarios match{}; restrict with --scenario to one of: {}",
            several.len(),
            filter.map(|sub| format!(" {sub:?}")).unwrap_or_default(),
            candidates
                .iter()
                .map(|report| report.meta.name.as_str())
                .collect::<Vec<_>>()
                .join(", "),
        )),
    }
}

/// Resolve the run at index `run` within `report`, bounds-checked with a friendly message. Pure, so it
/// is tested directly.
pub(crate) fn resolve_run(report: &ScenarioReport, run: usize) -> Result<&RunRecord, String> {
    report.runs.get(run).ok_or_else(|| {
        format!(
            "run {run} is out of range: scenario {:?} has {} run(s) (0..={})",
            report.meta.name,
            report.runs.len(),
            report.runs.len().saturating_sub(1),
        )
    })
}

fn names(scenarios: &[ScenarioReport]) -> String {
    scenarios
        .iter()
        .map(|report| report.meta.name.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}
