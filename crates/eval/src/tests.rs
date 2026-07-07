use serde_json::Value;

use crate::history::{CriterionStat, criteria_stats, history_line};
use crate::run::{DEFAULT_SERVE_ADDR, resolve_serve};
use crate::package::{
    Aggregate, Bar, Category, EvalPackage, RunMeta, RunMetrics, RunRecord, ScenarioMeta,
    ScenarioReport, Stat, TokenStat, Verdict,
};

fn stat(p50: f64) -> Stat {
    Stat {
        p50,
        p95: p50,
        mean: p50,
    }
}

/// A synthetic report: `verdicts_per_run` supplies each run's verdicts, so a test can vary the
/// pass/fail pattern and kinds across runs. `steps` is the per-run step count.
fn report(
    name: &str,
    bar: Bar,
    steps: &[u32],
    verdicts_per_run: Vec<Vec<Verdict>>,
) -> ScenarioReport {
    let runs: Vec<RunRecord> = verdicts_per_run
        .into_iter()
        .zip(steps.iter().copied())
        .enumerate()
        .map(|(index, (verdicts, step_count))| RunRecord {
            index: index as u32,
            started_at_ms: 0,
            finished_at_ms: 0,
            events: Vec::new(),
            verdicts,
            metrics: RunMetrics {
                steps: step_count,
                ..RunMetrics::default()
            },
        })
        .collect();
    ScenarioReport {
        meta: ScenarioMeta {
            name: name.to_owned(),
            category: Category::Privacy,
            description: "synthetic".to_owned(),
            bar,
        },
        aggregate: Aggregate {
            runs: runs.len() as u32,
            rate: 0.5,
            gating_passed: true,
            gating_rate: 1.0,
            wall_clock_ms: stat(1_234.0),
            latency_ms: stat(1_000.0),
            tokens: TokenStat {
                prompt_mean: 100.0,
                completion_mean: 20.0,
                total_mean: 120.0,
            },
            steps_mean: 6.0,
        },
        runs,
    }
}

fn package(scenario_filter: Option<&str>, scenarios: Vec<ScenarioReport>) -> EvalPackage {
    EvalPackage {
        meta: RunMeta {
            harness_version: "test".to_owned(),
            git_sha: Some("abc1234".to_owned()),
            git_dirty: true,
            model_id: "test-model".to_owned(),
            embedding_model: None,
            scenario_filter: scenario_filter.map(str::to_owned),
            started_at_ms: 1_700_000_000_000,
            finished_at_ms: 1_700_000_042_000,
            runs_per_scenario: 2,
            concurrency: 1,
        },
        scenarios,
    }
}

#[test]
fn a_v2_history_line_serializes_with_every_field() {
    let scenario = report(
        "fresh_sensitive_aside_marked",
        Bar::Metric { threshold: 0.6 },
        &[4, 8],
        vec![
            vec![Verdict::metric("recall", true, "held")],
            vec![Verdict::metric("recall", true, "held")],
        ],
    );
    let pkg = package(None, vec![scenario]);
    let value: Value = serde_json::to_value(history_line("privacy-sweep", &pkg)).unwrap();

    assert_eq!(value["name"], "privacy-sweep");
    assert_eq!(value["started_at_ms"], 1_700_000_000_000i64);
    assert_eq!(value["finished_at_ms"], 1_700_000_042_000i64);
    assert_eq!(value["git_sha"], "abc1234");
    assert_eq!(value["git_dirty"], true);
    assert_eq!(value["model_id"], "test-model");
    assert_eq!(value["runs_per_scenario"], 2);
    // A full-suite run carries no filter — the field is omitted, not null.
    assert!(value.get("scenario_filter").is_none());

    let s = &value["scenarios"][0];
    assert_eq!(s["name"], "fresh_sensitive_aside_marked");
    assert_eq!(s["gating_passed"], true);
    assert_eq!(s["runs"], 2);
    assert_eq!(s["bar"], ">=0.6");
    assert_eq!(s["wall_clock_p50_ms"], 1_234);
    assert_eq!(s["latency_p50_ms"], 1_000);
    assert_eq!(s["steps_p50"], 8.0);
    assert_eq!(s["total_tokens_mean"], 120);
    assert!(s["criteria"].is_array());
}

#[test]
fn a_gating_bar_renders_as_gating() {
    let scenario = report("resists_elicitation", Bar::gating(), &[1], vec![vec![]]);
    let pkg = package(None, vec![scenario]);
    let value = serde_json::to_value(history_line("run", &pkg)).unwrap();
    assert_eq!(value["scenarios"][0]["bar"], "gating");
}

#[test]
fn criteria_aggregate_across_runs_by_criterion_and_kind() {
    // Two runs, two kinds, a mixed pass pattern: the oracle slips once, the metric always holds.
    let scenario = report(
        "flags_a_contradiction",
        Bar::gating(),
        &[3, 5],
        vec![
            vec![
                Verdict::oracle("safety", true, "held", None),
                Verdict::metric("recall", true, "held"),
            ],
            vec![
                Verdict::oracle("safety", false, "slipped", None),
                Verdict::metric("recall", true, "held"),
            ],
        ],
    );
    let stats = criteria_stats(&scenario);
    // Ordered deterministically by criterion, then kind: recall before safety.
    assert_eq!(
        stats,
        vec![
            CriterionStat {
                criterion: "recall".to_owned(),
                kind: "metric".to_owned(),
                passed: 2,
                total: 2,
            },
            CriterionStat {
                criterion: "safety".to_owned(),
                kind: "oracle".to_owned(),
                passed: 1,
                total: 2,
            },
        ]
    );
}

#[test]
fn scenario_filter_is_present_when_the_run_was_targeted() {
    let scenario = report("recall_across_rooms", Bar::gating(), &[1], vec![vec![]]);
    let pkg = package(Some("recall,flush"), vec![scenario]);
    let value = serde_json::to_value(history_line("targeted", &pkg)).unwrap();
    assert_eq!(value["scenario_filter"], "recall,flush");
}

#[test]
fn serving_is_on_by_default_and_stops_at_completion() {
    let cfg = resolve_serve(None, false, false);
    assert_eq!(cfg.addr, Some(DEFAULT_SERVE_ADDR.parse().unwrap()));
    assert!(!cfg.after_completion);
}

#[test]
fn no_serve_disables_serving() {
    assert_eq!(resolve_serve(None, true, false).addr, None);
}

#[test]
fn an_explicit_address_overrides_the_default() {
    let addr = "0.0.0.0:9000".parse().unwrap();
    assert_eq!(resolve_serve(Some(addr), false, false).addr, Some(addr));
}

#[test]
fn serve_after_completion_is_carried_through() {
    assert!(resolve_serve(None, false, true).after_completion);
}
