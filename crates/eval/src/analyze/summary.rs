//! The summary view: per-scenario rates, bars, and deltas against a baseline.

use crate::{
    analyze::{bar_label, clears_bar, format::join_or_none},
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
