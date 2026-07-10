//! The failures view: a cross-scenario rollup of every missed verdict, then each failed run's
//! complete deliberation trace.

use zuihitsu::EventPayload;

use crate::package::{EvalPackage, RunRecord};

use super::format::trunc;

/// Every missed verdict across the (filtered) suite, rolled up by criterion — the cross-scenario view
/// that tells you what to work on next. A criterion that slips in several scenarios is one behavioral
/// thread, not N unrelated fails; grouping surfaces that. Each line gives the miss count, the scenarios
/// it appeared in, and one sample rationale. Sorted most-missed first.
fn print_rollup(pkg: &EvalPackage, scenario: Option<&str>) {
    /// One criterion's accumulated misses.
    struct Miss {
        criterion: String,
        count: usize,
        scenarios: Vec<String>,
        sample: String,
    }
    let mut misses: std::collections::BTreeMap<String, Miss> = std::collections::BTreeMap::new();
    for report in &pkg.scenarios {
        if scenario.is_some_and(|sub| !report.meta.name.contains(sub)) {
            continue;
        }
        for run in &report.runs {
            for verdict in &run.verdicts {
                if verdict.passed {
                    continue;
                }
                let entry = misses.entry(verdict.criterion.clone()).or_insert(Miss {
                    criterion: verdict.criterion.clone(),
                    count: 0,
                    scenarios: Vec::new(),
                    sample: verdict.rationale.trim().to_owned(),
                });
                entry.count += 1;
                if !entry.scenarios.iter().any(|s| s == &report.meta.name) {
                    entry.scenarios.push(report.meta.name.clone());
                }
            }
        }
    }
    let mut rows: Vec<Miss> = misses.into_values().collect();
    rows.sort_by_key(|m| std::cmp::Reverse(m.count));

    let total: usize = rows.iter().map(|m| m.count).sum();
    println!(
        "\n=== failure rollup — {total} missed verdict{} across {} criterion ===",
        if total == 1 { "" } else { "s" },
        rows.len(),
    );
    if rows.is_empty() {
        println!("  none");
        return;
    }
    for miss in &rows {
        println!("\n  [{}] {}", miss.count, miss.criterion);
        println!("    scenarios: {}", miss.scenarios.join(", "));
        if !miss.sample.is_empty() {
            println!("    e.g. {}", trunc(&miss.sample, 160));
        }
    }
}

pub(crate) fn print_failures(
    pkg: &EvalPackage,
    scenario: Option<&str>,
    events: Option<&str>,
    limit: usize,
    truncate: usize,
) {
    print_rollup(pkg, scenario);

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
            print_run(run, events, truncate);
        }
    }
    if !shown {
        println!(
            "\n(no failing runs to dump{})",
            match scenario {
                Some(sub) => format!(" matching '{sub}'"),
                None => String::new(),
            }
        );
    }
}

fn print_run(run: &RunRecord, events: Option<&str>, truncate: usize) {
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
    if let Some(substr) = events {
        print_events(run, substr, truncate);
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

/// The events in `run` whose payload type contains `substr`, each as a one-line field summary — the
/// per-run diagnostic for pinpointing why a run failed at the event level (a wake-up that never fired,
/// an `occurred_at` that landed null or malformed, a description regenerated). Empty when none match.
fn print_events(run: &RunRecord, substr: &str, truncate: usize) {
    let lines: Vec<String> = run
        .events
        .iter()
        .filter_map(|event| summarize_event(&event.payload, substr))
        .map(|line| trunc(&line, truncate))
        .collect();
    if lines.is_empty() {
        return;
    }
    println!("  ── events [{substr}] ──");
    for line in lines {
        println!("    {line}");
    }
}

/// A compact one-line summary of a diagnostic event whose type name contains `substr`, or `None` — the
/// substring gate for the `--events` lens, delegating the rendering to the shared
/// [`diagnostic_summary`](crate::event_render::diagnostic_summary) so the two event views stay
/// consistent. A payload whose type name does not contain `substr`, or which carries no diagnostic
/// signal, filters out.
fn summarize_event(payload: &EventPayload, substr: &str) -> Option<String> {
    if !crate::event_render::payload_type(payload).contains(substr) {
        return None;
    }
    crate::event_render::diagnostic_summary(payload)
}
