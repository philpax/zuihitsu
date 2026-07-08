use std::time::{SystemTime, UNIX_EPOCH};

use super::{EvalSink, LiveEvent, read_sidecar};
use crate::package::{Bar, Category, RunMeta, RunMetrics, RunRecord, ScenarioMeta};

/// A pre-timing sidecar predates `at_ms` on the run boundaries; `#[serde(default)]` must fill `0`
/// so an old line still folds.
#[test]
fn old_run_boundary_lines_default_at_ms_to_zero() {
    let started: LiveEvent = serde_json::from_str(r#"{"kind":"run_started","scenario":0,"run":2}"#)
        .expect("old run_started parses");
    match started {
        LiveEvent::RunStarted { run, at_ms, .. } => {
            assert_eq!(run, 2);
            assert_eq!(at_ms, 0);
        }
        other => panic!("expected RunStarted, got {other:?}"),
    }
}

/// The stamping seam: a run driven through the sink carries its wall-clock stamps into the package
/// and onto the `RunCompleted`'s `at_ms`, and survives the sidecar resume round-trip.
#[test]
fn a_stamped_run_survives_the_sidecar_and_resume() {
    let dir = std::env::temp_dir().join(format!(
        "zuihitsu-eval-live-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let sidecar = dir.join("run.jsonl");

    let meta = RunMeta {
        harness_version: "test".to_owned(),
        git_sha: None,
        git_dirty: false,
        model_id: "test-model".to_owned(),
        embedding_model: None,
        scenario_filter: None,
        started_at_ms: 100,
        finished_at_ms: 100,
        runs_per_scenario: 1,
        concurrency: 1,
    };
    let scenario = ScenarioMeta {
        name: "seam".to_owned(),
        category: Category::Recall,
        description: "seam test".to_owned(),
        bar: Bar::gating(),
    };
    let sink = EvalSink::new(meta, vec![scenario], &sidecar).expect("sink opens");

    sink.run_started(0, 0, 1_000).unwrap();
    let record = RunRecord {
        index: 0,
        started_at_ms: 1_000,
        finished_at_ms: 5_000,
        events: Vec::new(),
        verdicts: Vec::new(),
        metrics: RunMetrics::default(),
    };
    sink.run_finished(0, record).unwrap();

    let run = &sink.package().scenarios[0].runs[0];
    assert_eq!(run.started_at_ms, 1_000);
    assert_eq!(run.finished_at_ms, 5_000);

    let resumed = read_sidecar(&sidecar).expect("resume reads the sidecar");
    let (_, resumed_run) = &resumed.completed[0];
    assert_eq!(resumed_run.started_at_ms, 1_000);
    assert_eq!(resumed_run.finished_at_ms, 5_000);

    std::fs::remove_dir_all(&dir).ok();
}
