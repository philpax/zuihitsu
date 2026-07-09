//! `eval analyze` — read an eval package at the terminal. The console renders a package richly, but to
//! judge a run and decide the next prompt/code edit you often just want two things at a prompt: how each
//! scenario moved against a baseline, and the *complete deliberation* of the runs that failed — the
//! agent's per-step reasoning, the Lua it emitted with its results, and which oracle it missed and why.
//! This is the command-line counterpart to the viewer, typed directly against the package contract.

mod failures;
pub(crate) mod format;
mod relations;
mod summary;

#[cfg(test)]
mod tests;

use std::{fs, path::Path};

use crate::{
    error::EvalError,
    package::{Bar, EvalPackage, ScenarioReport},
};

pub(crate) use failures::print_failures;
pub(crate) use relations::print_relations;
pub(crate) use summary::print_summary;

// Re-export items the test module reaches for through `super::`.
#[cfg(test)]
pub(crate) use format::render_locations;
#[cfg(test)]
pub(crate) use relations::{project_relations, render_shapes};

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

pub(crate) fn load(path: &Path) -> Result<EvalPackage, EvalError> {
    let text = fs::read_to_string(path).map_err(|source| EvalError::ReadPackage {
        path: path.to_path_buf(),
        source,
    })?;
    serde_json::from_str(&text).map_err(|source| EvalError::LoadPackage {
        path: path.to_path_buf(),
        source: Box::new(source),
    })
}

pub(crate) fn bar_label(bar: &Bar) -> String {
    match bar {
        Bar::Gating { min_rate } if *min_rate >= 1.0 => "gate".to_owned(),
        Bar::Gating { min_rate } => format!("gate>={min_rate}"),
        Bar::Metric { threshold } => format!(">={threshold}"),
    }
}

/// Whether a scenario's aggregate clears its bar — a held gate, a rate at or above a rate gate's
/// threshold, or a metric rate at or above its reporting threshold.
pub(crate) fn clears_bar(report: &ScenarioReport) -> bool {
    match report.meta.bar {
        bar @ Bar::Gating { .. } => {
            bar.holds(report.aggregate.gating_rate, report.aggregate.gating_passed)
        }
        Bar::Metric { threshold } => report.aggregate.rate >= threshold,
    }
}
