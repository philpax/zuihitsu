//! The `status` subcommand: what a run has actually done, read from its `.jsonl` sidecar.
//!
//! A suite run is long enough that "how far along is it, and is it still moving?" is a question asked
//! far more often than it is answerable. The run's log does not answer it — the per-scenario result
//! lines print in one batch when the suite finishes, so a log that has scrolled for hours can hold no
//! progress at all, and its tail shows only whatever the last turn happened to emit. The sidecar is
//! the authoritative record: one `RunCompleted` per finished run, written as the run goes. This
//! reports it, so progress and liveness are one command rather than an inference from log shape.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{
    error::EvalError,
    live::read_sidecar,
    package::{EvalPackage, RunRecord, VerdictKind},
};

/// How long a sidecar may go without a new completed run before the run is called stalled rather than
/// working. Comfortably above the slowest scenario's wall-clock (the sessions group runs to a few
/// minutes a run), so a slow scenario is never mistaken for a wedged backend.
const STALL_AFTER_MS: i64 = 15 * 60 * 1000;

/// Report a run's progress. `target` is a run name (`2026-07-26-full-n5`), or a path to either the
/// sidecar or the finished package; a bare name resolves under `eval/`. `None` picks the most
/// recently written sidecar, so checking on the run in flight needs no name at hand.
pub fn report(target: Option<&str>) -> Result<(), EvalError> {
    let target = match target {
        Some(target) => target.to_owned(),
        None => match newest_sidecar()? {
            Some(name) => name,
            None => {
                println!("no run in flight — no sidecar under eval/");
                return Ok(());
            }
        },
    };
    let target = target.as_str();
    let (sidecar, package) = resolve(target);
    if !sidecar.exists() {
        if package.exists() {
            return report_finished(&package);
        }
        println!(
            "no run named {target:?} — expected {} or {}",
            sidecar.display(),
            package.display()
        );
        return Ok(());
    }
    let state = read_sidecar(&sidecar)?;
    let total = state.scenarios.len() as u32 * state.meta.runs_per_scenario.max(1);
    let done = state.completed.len() as u32;

    // Per-scenario completion, by the scenario's index into the manifest — the same key the sidecar
    // records against, so a scenario is "done" only at its full run count.
    let mut per_scenario = vec![0u32; state.scenarios.len()];
    for (scenario, _) in &state.completed {
        if let Some(slot) = per_scenario.get_mut(*scenario as usize) {
            *slot += 1;
        }
    }
    let full = state.meta.runs_per_scenario.max(1);
    let complete = per_scenario.iter().filter(|count| **count >= full).count();
    let remaining: Vec<&str> = state
        .scenarios
        .iter()
        .zip(&per_scenario)
        .filter(|(_, count)| **count < full)
        .map(|(scenario, _)| scenario.name.as_str())
        .collect();

    let now = now_ms();
    let elapsed = now.saturating_sub(state.meta.started_at_ms);
    println!("{target} — {}", liveness(state.last_completed_at_ms, now));
    println!(
        "  started   {} ago{}",
        humane(elapsed),
        match &state.meta.scenario_filter {
            Some(filter) => format!("  (--scenario {filter})"),
            None => "  (full suite)".to_owned(),
        }
    );
    println!(
        "  progress  {done}/{total} runs · {complete}/{} scenarios at N={full}",
        state.scenarios.len()
    );
    if done > 0 && done < total {
        // Extrapolate from what this run has actually managed, not from a historical average: a
        // resumed run's elapsed covers only the runs it drove, which is the rate that matters.
        let per_run = elapsed / i64::from(done);
        println!(
            "  remaining {} scenarios, ~{} at the observed rate",
            remaining.len(),
            humane(per_run * i64::from(total - done))
        );
    }
    if !remaining.is_empty() {
        println!("  next      {}", preview(&remaining));
    }
    report_interim(&state.completed, &state.scenarios);
    Ok(())
}

/// The verdicts so far, so a run that is going wrong is visible while it still has hours to go rather
/// than only in the final package. A scenario's *bar* decides whether a missed oracle fails the suite,
/// not the verdict's kind: an `Oracle` verdict under a `Bar::Metric` scenario is tracked, never gating.
/// Only a would-fail is called out in alarm terms; the weakest rates follow, since a rate that has
/// already collapsed is usually the reason to stop a run early rather than let it finish.
fn report_interim(completed: &[(u32, RunRecord)], scenarios: &[crate::package::ScenarioMeta]) {
    let mut oracles: BTreeMap<u32, (u32, u32)> = BTreeMap::new();
    let mut metrics: BTreeMap<u32, (u32, u32)> = BTreeMap::new();
    for (scenario, record) in completed {
        for verdict in &record.verdicts {
            let bucket = match verdict.kind {
                VerdictKind::Oracle => oracles.entry(*scenario).or_default(),
                VerdictKind::Metric => metrics.entry(*scenario).or_default(),
            };
            bucket.1 += 1;
            if verdict.passed {
                bucket.0 += 1;
            }
        }
    }
    let name = |index: &u32| {
        scenarios
            .get(*index as usize)
            .map(|scenario| scenario.name.as_str())
            .unwrap_or("?")
    };
    // A gating bar's verdict rate so far, judged by the bar itself — `holds` reads the pass/pass-rate
    // exactly as the harness's own exit-code check will at the end.
    let failing: Vec<String> = oracles
        .iter()
        .filter(|(index, (passed, total))| {
            scenarios.get(**index as usize).is_some_and(|scenario| {
                let rate = f64::from(*passed) / f64::from((*total).max(1));
                !scenario.bar.holds(rate, passed == total)
            })
        })
        .map(|(index, (passed, total))| format!("{} ({passed}/{total})", name(index)))
        .collect();
    if failing.is_empty() {
        println!("  gating    every gating bar holds so far");
    } else {
        println!(
            "  gating    WOULD FAIL — {}: {}",
            failing.len(),
            failing.join(", ")
        );
    }
    // Every sub-1.0 rate, gating and metric alike, weakest first — a tracked metric sliding is worth
    // seeing mid-run even though it cannot fail the suite.
    let mut weakest: Vec<(&str, f64, u32, u32)> = oracles
        .iter()
        .chain(metrics.iter())
        .map(|(index, (passed, total))| {
            let rate = f64::from(*passed) / f64::from((*total).max(1));
            (name(index), rate, *passed, *total)
        })
        .filter(|(_, rate, _, _)| *rate < 1.0)
        .collect();
    weakest.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(b.0)));
    weakest.dedup_by(|a, b| a.0 == b.0);
    if !weakest.is_empty() {
        let shown: Vec<String> = weakest
            .iter()
            .take(4)
            .map(|(name, rate, passed, total)| format!("{name} {rate:.2} ({passed}/{total})"))
            .collect();
        println!("  weakest   {}", shown.join(", "));
    }
}

/// The most recently written `.jsonl` under `eval/` — the run in flight, when there is one.
fn newest_sidecar() -> Result<Option<String>, EvalError> {
    let dir = Path::new("eval");
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(None);
    };
    let mut newest: Option<(std::time::SystemTime, String)> = None;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|ext| ext != "jsonl") {
            continue;
        }
        // history.jsonl is the tracked trend record, not a run.
        let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        if stem.starts_with("history") {
            continue;
        }
        let Ok(modified) = entry.metadata().and_then(|meta| meta.modified()) else {
            continue;
        };
        if newest.as_ref().is_none_or(|(seen, _)| modified > *seen) {
            newest = Some((modified, stem.to_owned()));
        }
    }
    Ok(newest.map(|(_, stem)| stem))
}

/// A finished package has no sidecar — the run folded it away on completion — so its status is simply
/// that it is done, with the span it took.
fn report_finished(package: &Path) -> Result<(), EvalError> {
    let text = std::fs::read_to_string(package).map_err(|source| EvalError::WriteOutput {
        path: package.to_path_buf(),
        source,
    })?;
    let parsed: EvalPackage = serde_json::from_str(&text)?;
    let span = parsed.meta.finished_at_ms - parsed.meta.started_at_ms;
    println!("{} — finished", package.display());
    println!(
        "  took      {}  ({} scenarios at N={})",
        humane(span),
        parsed.scenarios.len(),
        parsed.meta.runs_per_scenario
    );
    println!("  analyze   eval analyze {}", package.display());
    Ok(())
}

/// Whether the run is still producing, phrased from how long ago its last run landed. A sidecar whose
/// newest record is older than [`STALL_AFTER_MS`] is called stalled: the usual cause is the model
/// backend having gone away, where the process stays alive retrying and the log fills with backoff
/// while nothing completes.
fn liveness(last_completed_at_ms: Option<i64>, now: i64) -> String {
    let Some(last) = last_completed_at_ms else {
        return "no runs completed yet".to_owned();
    };
    let since = now.saturating_sub(last);
    if since > STALL_AFTER_MS {
        format!(
            "STALLED — nothing completed in {} (is the model backend up?)",
            humane(since)
        )
    } else {
        format!("running — last run completed {} ago", humane(since))
    }
}

/// Resolve a run name or path to its `(sidecar, package)` pair. A bare name lives under `eval/`; a
/// path is taken as given, with either extension accepted.
fn resolve(target: &str) -> (PathBuf, PathBuf) {
    let base = if target.contains('/') || target.contains('\\') {
        PathBuf::from(target)
    } else {
        PathBuf::from("eval").join(target)
    };
    (base.with_extension("jsonl"), base.with_extension("json"))
}

/// The first few remaining scenario names, so the line stays readable when many are left.
fn preview(remaining: &[&str]) -> String {
    const SHOWN: usize = 4;
    let head = remaining.iter().take(SHOWN).copied().collect::<Vec<_>>();
    match remaining.len().checked_sub(SHOWN) {
        Some(rest) if rest > 0 => format!("{} (+{rest} more)", head.join(", ")),
        _ => head.join(", "),
    }
}

/// A duration in milliseconds as a coarse human span (`4h 20m`, `12m`, `45s`).
fn humane(ms: i64) -> String {
    let seconds = ms.max(0) / 1000;
    let (hours, minutes) = (seconds / 3600, (seconds % 3600) / 60);
    if hours > 0 {
        format!("{hours}h {minutes}m")
    } else if minutes > 0 {
        format!("{minutes}m")
    } else {
        format!("{seconds}s")
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_millis() as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quiet_sidecar_reads_as_stalled_not_slow() {
        // The distinction the whole subcommand exists for: a process that is alive but completing
        // nothing looks identical to a working one from the outside, and its log fills with retries.
        let now = 10 * 60 * 60 * 1000;
        let working = liveness(Some(now - 60 * 1000), now);
        assert!(working.contains("running"), "{working}");
        let wedged = liveness(Some(now - STALL_AFTER_MS - 1), now);
        assert!(wedged.contains("STALLED"), "{wedged}");
        assert!(wedged.contains("backend"), "{wedged}");
    }

    #[test]
    fn a_sidecar_with_no_completions_is_neither_running_nor_stalled() {
        // A run whose first scenario has not finished has completed nothing, which is ordinary at the
        // start and says nothing either way about health.
        let report = liveness(None, 10_000);
        assert!(report.contains("no runs completed"), "{report}");
    }

    #[test]
    fn a_bare_name_resolves_under_the_eval_directory() {
        let (sidecar, package) = resolve("2026-07-26-full-n5");
        assert_eq!(sidecar, PathBuf::from("eval/2026-07-26-full-n5.jsonl"));
        assert_eq!(package, PathBuf::from("eval/2026-07-26-full-n5.json"));
    }

    #[test]
    fn a_path_is_taken_as_given_under_either_extension() {
        for given in ["out/run.jsonl", "out/run.json"] {
            let (sidecar, package) = resolve(given);
            assert_eq!(sidecar, PathBuf::from("out/run.jsonl"));
            assert_eq!(package, PathBuf::from("out/run.json"));
        }
    }

    #[test]
    fn the_remaining_preview_caps_and_counts_the_rest() {
        assert_eq!(preview(&["a", "b"]), "a, b");
        assert_eq!(preview(&["a", "b", "c", "d"]), "a, b, c, d");
        assert_eq!(preview(&["a", "b", "c", "d", "e"]), "a, b, c, d (+1 more)");
    }

    #[test]
    fn a_span_renders_coarsely() {
        assert_eq!(humane(45_000), "45s");
        assert_eq!(humane(12 * 60 * 1000), "12m");
        assert_eq!(humane((4 * 3600 + 20 * 60) * 1000), "4h 20m");
        assert_eq!(humane(-1), "0s");
    }
}
