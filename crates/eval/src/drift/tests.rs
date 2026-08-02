//! Drift detection over synthetic history lines — the arithmetic is exercised without a history file
//! on disk, the way [`crate::history::project`] is split from its file-reading caller.

use super::*;
use crate::history::HistoryLine;

/// The one `(scenario, criterion)` pair every fixture line carries.
fn measured() -> BTreeSet<(String, String)> {
    BTreeSet::from([("a_scenario".to_owned(), "a_criterion".to_owned())])
}

/// A history line carrying one scenario with one criterion at `(passed, total)`, built through the
/// deserializer rather than the struct literal: the record's fields stay private to its own module,
/// and a fixture that goes in as JSON exercises the same shape the tracked history is read from.
fn line(name: &str, sha: &str, passed: u32, total: u32) -> HistoryLine {
    serde_json::from_value(serde_json::json!({
        "name": name,
        "started_at_ms": 0,
        "finished_at_ms": 0,
        "git_sha": sha,
        "git_dirty": false,
        "model_id": "test",
        "runs_per_scenario": total,
        "scenarios": [{
            "name": "a_scenario",
            "rate": rate((passed, total)),
            "gating_passed": true,
            "runs": total,
            "bar": "gate",
            "wall_clock_p50_ms": 0,
            "latency_p50_ms": 0,
            "steps_p50": 0.0,
            "total_tokens_mean": 0,
            "criteria": [{
                "criterion": "a_criterion",
                "kind": "oracle",
                "passed": passed,
                "total": total,
            }],
        }],
    }))
    .expect("the fixture matches the history record shape")
}

#[test]
fn a_steady_criterion_does_not_drift() {
    let history: Vec<HistoryLine> = (0..8)
        .map(|i| line(&format!("run{i}"), "aaa", 5, 5))
        .collect();
    assert!(detect(&history, &measured()).is_empty());
}

#[test]
fn a_pooled_fall_is_flagged_where_neither_run_alone_would_be() {
    // The shape this exists for: a criterion long held at ~0.92 whose rate has really fallen. The first
    // run after the change comes up 5/5 — indistinguishable from the old rate — and only the second,
    // at 1/5, disagrees. Pooled the two are 6/10 against 44/48, which is decisive.
    let mut history: Vec<HistoryLine> = Vec::new();
    for i in 0..6 {
        history.push(line(&format!("old{i}"), "before", 8, 8));
    }
    history.push(line("after1", "suspect", 5, 5));
    history.push(line("after2", "suspect", 1, 5));

    let drifts = detect(&history, &measured());
    assert_eq!(drifts.len(), 1, "{drifts:?}");
    let drift = &drifts[0];
    assert_eq!(drift.recent, (6, 10));
    assert_eq!(drift.prior, (48, 48));
    assert!(drift.fell());
    assert!(drift.p_value < 0.01, "p = {}", drift.p_value);
    // The bracket names the commit the drift is between, which is the point of recording it.
    assert_eq!(drift.since, "before");
    assert_eq!(drift.until, "suspect");
}

#[test]
fn a_single_unlucky_run_alone_is_not_enough() {
    // The same 1/5, but with the run before it still at the old rate rather than also fallen, against a
    // criterion that has always slipped occasionally. Pooled the recent window is 9/13, which a 0.875
    // prior explains, so nothing is flagged — the check waits for a second run to agree rather than
    // firing on one bad draw.
    let mut history: Vec<HistoryLine> = (0..6)
        .map(|i| line(&format!("old{i}"), "before", 7, 8))
        .collect();
    history.push(line("after1", "suspect", 8, 8));
    history.push(line("after2", "suspect", 1, 5));
    let drifts = detect(&history, &measured());
    assert!(drifts.is_empty(), "{drifts:?}");
}

#[test]
fn a_rise_is_flagged_too() {
    // A criterion that suddenly cannot fail is as much a signal as one that started failing: a fixture
    // that stopped exercising the path it names reports as a perfect rate.
    let mut history: Vec<HistoryLine> = (0..6)
        .map(|i| line(&format!("old{i}"), "before", 4, 8))
        .collect();
    history.push(line("after1", "suspect", 8, 8));
    history.push(line("after2", "suspect", 8, 8));
    let drifts = detect(&history, &measured());
    assert_eq!(drifts.len(), 1, "{drifts:?}");
    assert!(!drifts[0].fell());
}

#[test]
fn a_re_run_name_supersedes_its_earlier_row() {
    // A heal (`--retry-infra-failed`) appends a fresh row under the same run name. Counting both would
    // double that run's weight and let the tallies the heal exists to supersede pull the prior around.
    let mut history: Vec<HistoryLine> = (0..6)
        .map(|i| line(&format!("old{i}"), "before", 8, 8))
        .collect();
    history.push(line("healed", "suspect", 0, 5)); // the poisoned row
    history.push(line("healed", "suspect", 5, 5)); // the heal, same name
    history.push(line("after", "suspect", 5, 5));
    let drifts = detect(&history, &measured());
    assert!(
        drifts.is_empty(),
        "the superseded row must not count: {drifts:?}"
    );
}

#[test]
fn a_thin_history_is_not_judged() {
    // Below the prior floor the baseline rate is itself a guess, and comparing against it manufactures
    // signal from noise.
    let history = vec![
        line("a", "x", 2, 2),
        line("b", "x", 2, 2),
        line("c", "x", 0, 2),
        line("d", "x", 0, 2),
    ];
    assert!(detect(&history, &measured()).is_empty());
}

#[test]
fn a_statistically_significant_but_tiny_move_is_not_reported() {
    // A long history makes a trivial shift significant. True, and useless to act on.
    let mut history: Vec<HistoryLine> = (0..20)
        .map(|i| line(&format!("old{i}"), "before", 98, 100))
        .collect();
    history.push(line("after1", "suspect", 92, 100));
    history.push(line("after2", "suspect", 92, 100));
    let drifts = detect(&history, &measured());
    assert!(drifts.is_empty(), "{drifts:?}");
}

#[test]
fn the_binomial_tail_matches_a_hand_computed_case() {
    // P(X <= 1) for X ~ Binomial(5, 0.9) = 0.1^5 + 5 * 0.9 * 0.1^4 = 0.00001 + 0.00045.
    let p = tail_probability((1, 5), 0.9);
    assert!((p - 0.00046).abs() < 1e-9, "got {p}");
    // The upper tail of a perfect result under a middling rate: P(X >= 8) for Binomial(8, 0.5).
    let p = tail_probability((8, 8), 0.5);
    assert!((p - 0.5_f64.powi(8)).abs() < 1e-12, "got {p}");
}

#[test]
fn the_org_namespace_drift_is_caught_two_runs_after_it_lands() {
    // The motivating case, with the real tallies this criterion recorded. Adding the org/ namespace on
    // 26 July moved its true rate from ~0.92 to ~0.65, and nothing noticed for a week: the first run
    // after the change came up 5/5, which the old rate explains perfectly well.
    //
    // The run before the change (`00fe7db`) and the one after (`ac19840`) are both 5/5, so a check that
    // read either alone sees nothing. Pooled with the 1/5 that follows, the window is 6/10 against a
    // long 0.92 prior, and the bracket names the pair of commits the change sits between.
    let history = vec![
        line("2026-07-12-full-n10", "b19ccc6", 10, 10),
        line("2026-07-13-full-n10", "c0a2bca", 8, 10),
        line("2026-07-19-full-n3", "2ff288f", 3, 3),
        line("2026-07-20-full-n10-post-101", "5c01df0", 8, 10),
        line("2026-07-24-post-canonical-identity", "a9a1ea6", 5, 5),
        line("2026-07-26-full-n5", "00fe7db", 5, 5),
        // org/ lands here.
        line("2026-07-26-post-review-fixes", "ac19840", 5, 5),
        line("2026-08-02-span-justification", "dcb918c", 1, 5),
    ];

    // Reading only up to the first post-change run: nothing to see, correctly.
    assert!(
        detect(&history[..7], &measured()).is_empty(),
        "5/5 after the change is indistinguishable from the old rate",
    );

    // One more run, and it is decisive.
    let drifts = detect(&history, &measured());
    assert_eq!(drifts.len(), 1, "{drifts:?}");
    let drift = &drifts[0];
    assert!(drift.fell());
    assert_eq!(drift.recent, (6, 10));
    // p ~ 0.018: real evidence rather than the decisive number the rates alone suggest, and the reason
    // the threshold sits at 0.05 — a tidier 0.01 would read as more rigorous and miss this entirely.
    assert!(
        (0.015..0.02).contains(&drift.p_value),
        "p = {}",
        drift.p_value
    );
    assert_eq!(drift.since, "00fe7db", "the last run before the window");
    assert_eq!(drift.until, "ac19840", "the first run within it");
}
