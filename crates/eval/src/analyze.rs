//! `eval analyze` — read an eval package at the terminal. The console renders a package richly, but to
//! judge a run and decide the next prompt/code edit you often just want two things at a prompt: how each
//! scenario moved against a baseline, and the *complete deliberation* of the runs that failed — the
//! agent's per-step reasoning, the Lua it emitted with its results, and which oracle it missed and why.
//! This is the command-line counterpart to the viewer, typed directly against the package contract.

use std::{fs, path::Path};

use zuihitsu::EventPayload;

use crate::{
    error::EvalError,
    package::{Bar, EvalPackage, RunRecord, ScenarioReport},
};

/// Print the summary (the default), or — with `failures` — dump the failed runs' deliberation traces.
/// `scenario` restricts both modes to scenarios whose name contains the substring; `limit` caps the
/// failed runs dumped per scenario (`0` = all); `truncate` clips long reasoning/scripts (`0` = full).
pub fn analyze(
    package: &Path,
    baseline: Option<&Path>,
    failures: bool,
    scenario: Option<&str>,
    limit: usize,
    truncate: usize,
) -> Result<(), EvalError> {
    let pkg = load(package)?;
    if failures {
        print_failures(&pkg, scenario, limit, truncate);
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
        source,
    })
}

fn bar_label(bar: &Bar) -> String {
    match bar {
        Bar::Gating => "gate".to_owned(),
        Bar::Metric { threshold } => format!(">={threshold}"),
    }
}

/// Whether a scenario's aggregate clears its bar — a held gate, or a rate at or above the threshold.
fn clears_bar(report: &ScenarioReport) -> bool {
    match report.meta.bar {
        Bar::Gating => report.aggregate.gating_passed,
        Bar::Metric { threshold } => report.aggregate.rate >= threshold,
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

fn print_failures(pkg: &EvalPackage, scenario: Option<&str>, limit: usize, truncate: usize) {
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
            print_run(run, truncate);
        }
    }
    if !shown {
        match scenario {
            Some(sub) => println!("no failing runs matching '{sub}'"),
            None => println!("no failing runs"),
        }
    }
}

fn print_run(run: &RunRecord, truncate: usize) {
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
