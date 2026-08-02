//! The summary view: per-scenario rates, bars, and deltas against a baseline.

use crate::{
    analyze::{bar_label, clears_bar, format::join_or_none, gate_held},
    package::{EvalPackage, ScenarioReport},
};

pub(crate) fn print_summary(pkg: &EvalPackage, base: Option<&EvalPackage>, scenario: Option<&str>) {
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
            if gate_held(r) { "ok" } else { "FAIL" },
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
        .filter(|r| !gate_held(r))
        .map(|r| r.meta.name.as_str())
        .collect();
    let below: Vec<&str> = reports
        .iter()
        .filter(|r| !clears_bar(r))
        .map(|r| r.meta.name.as_str())
        .collect();
    println!("\ngating not held: {}", join_or_none(&gate_fail));
    println!("below bar:       {}", join_or_none(&below));

    // A metric scenario's gating verdicts are still worth seeing when one misses, but it never fails
    // the suite, so it is reported apart from the bar-driven lines rather than inside them.
    let missed_under_metric: Vec<&str> = reports
        .iter()
        .filter(|r| !r.aggregate.gating_passed && gate_held(r))
        .map(|r| r.meta.name.as_str())
        .collect();
    if !missed_under_metric.is_empty() {
        println!(
            "gating verdicts missed, but the bar tolerates it: {}",
            join_or_none(&missed_under_metric),
        );
    }

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

    print_drift(pkg, scenario);
}

/// Report criteria whose recent rate no longer looks like a sample from their own history, pooled
/// across the trailing runs and bracketed to a commit range. Advisory: a deliberate improvement drifts
/// exactly as loudly as a regression, so this never touches the exit code — it says where to look.
///
/// Distinct from the `regressed`/`improved` lines above, which compare *this* run against one chosen
/// baseline. A baseline delta answers "did it move since that run"; drift answers "is this still the
/// same scenario it has been", which is the question a bar cannot ask and a single run cannot answer.
fn print_drift(pkg: &EvalPackage, scenario: Option<&str>) {
    // Anchored to what *this* package measured, so a criterion retired long ago cannot report its last
    // runs as current news.
    let measured: std::collections::BTreeSet<(String, String)> = pkg
        .scenarios
        .iter()
        .flat_map(|report| {
            let name = report.meta.name.clone();
            report
                .runs
                .iter()
                .flat_map(|run| run.verdicts.iter())
                .map(move |v| (name.clone(), v.criterion.clone()))
        })
        .collect();
    let history = crate::history::read_all();
    let drifts: Vec<_> = crate::drift::detect(&history, &measured)
        .into_iter()
        .filter(|d| scenario.is_none_or(|s| d.scenario.contains(s)))
        .collect();
    if drifts.is_empty() {
        return;
    }
    println!("\ndrift vs history (advisory, not gating):");
    for d in &drifts {
        let bracket = match (d.since.is_empty(), d.until.is_empty()) {
            (false, false) if d.since != d.until => format!(" between {}..{}", d.since, d.until),
            (false, false) => format!(" at {}", d.until),
            _ => String::new(),
        };
        println!(
            "  {} {:.2} -> {:.2} ({}/{} -> {}/{}, p={:.4}){}\n    {}",
            if d.fell() { "fell " } else { "rose " },
            crate::drift::rate(d.prior),
            crate::drift::rate(d.recent),
            d.prior.0,
            d.prior.1,
            d.recent.0,
            d.recent.1,
            d.p_value,
            bracket,
            format_args!("{} / {}", d.scenario, d.criterion),
        );
    }
}
