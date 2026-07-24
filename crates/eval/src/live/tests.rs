use std::time::{SystemTime, UNIX_EPOCH};

use zuihitsu::{
    Completion, ConversationId, Event, EventPayload, EventSource, ModelPhase, Seq, Timestamp,
    TurnId, Usage,
};

use crate::{
    live::{EvalSink, LiveEvent, read_sidecar},
    package::{Bar, Category, RunMeta, RunMetrics, RunRecord, ScenarioMeta},
};

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
        rejudged_from: None,
        resumed_from: None,
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
        journal: Vec::new(),
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

/// Progress frames broadcast to live subscribers but never persist: the sidecar and the folded
/// package are identical with or without them, so replay and resume are unaffected by who watched.
#[test]
fn run_progress_broadcasts_without_persisting() {
    let dir = std::env::temp_dir().join(format!(
        "zuihitsu-progress-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let sidecar = dir.join("progress.jsonl");
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
        rejudged_from: None,
        resumed_from: None,
    };
    let scenario = ScenarioMeta {
        name: "progress".to_owned(),
        category: Category::Recall,
        description: "progress seam".to_owned(),
        bar: Bar::gating(),
    };
    let sink = EvalSink::new(meta, vec![scenario], &sidecar).unwrap();
    let (_, _, mut receiver) = sink.subscribe();

    let before = std::fs::read_to_string(&sidecar).unwrap_or_default();
    sink.run_progress(
        0,
        0,
        zuihitsu::progress::TurnProgress {
            conversation: zuihitsu::ConversationId::generate(),
            turn_id: zuihitsu::TurnId::generate(),
            phase: zuihitsu::event::ModelPhase::Step,
            kind: zuihitsu::progress::ProgressKind::Reply,
            text: "Hel".to_owned(),
            step: 0,
        },
    );
    let (_, event) = receiver.try_recv().expect("the frame broadcasts");
    assert!(matches!(event, LiveEvent::RunProgress { frame, .. } if frame.text == "Hel"));
    let after = std::fs::read_to_string(&sidecar).unwrap_or_default();
    assert_eq!(before, after, "nothing lands in the sidecar");
}

/// A unique temp directory for a filesystem-touching test.
fn temp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "zuihitsu-eval-live-{tag}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn meta() -> RunMeta {
    RunMeta {
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
        rejudged_from: None,
        resumed_from: None,
    }
}

fn scenario_meta(name: &str) -> ScenarioMeta {
    ScenarioMeta {
        name: name.to_owned(),
        category: Category::Recall,
        description: "summary seam".to_owned(),
        bar: Bar::gating(),
    }
}

/// A `ModelCalled` event carrying the given usage — enough for the summary extraction to pick up.
fn model_called(seq: u64, usage: Usage) -> Event {
    Event {
        seq: Seq(seq),
        recorded_at: Timestamp::from_millis(seq as i64),
        source: EventSource::Agent,
        payload: EventPayload::ModelCalled {
            conversation: ConversationId::generate(),
            turn_id: TurnId::generate(),
            phase: ModelPhase::Step,
            request_digest: "digest".to_owned(),
            request: None,
            completion: Completion::Reply("hi".to_owned()),
            reasoning: None,
            finish_reason: None,
            usage,
            duration_ms: 0,
        },
    }
}

fn usage(prompt: u32, cache_read: u32) -> Usage {
    Usage {
        prompt_tokens: Some(prompt),
        completion_tokens: Some(1),
        total_tokens: Some(prompt + 1),
        cache_read_tokens: Some(cache_read),
        cache_write_tokens: None,
    }
}

/// A run's completion writes the authoritative full `RunCompleted` to the sidecar but broadcasts only
/// the lean `RunSummarized` — no full record ever crosses the wire.
#[test]
fn run_finished_writes_the_full_record_but_broadcasts_a_summary() {
    let dir = temp_dir("summary");
    let sidecar = dir.join("summary.jsonl");
    let sink = EvalSink::new(meta(), vec![scenario_meta("seam")], &sidecar).expect("sink opens");
    let (_, _, mut receiver) = sink.subscribe();

    let record = RunRecord {
        index: 0,
        started_at_ms: 1_000,
        finished_at_ms: 5_000,
        events: vec![
            model_called(1, usage(100, 90)),
            model_called(2, usage(200, 20)),
        ],
        journal: Vec::new(),
        verdicts: Vec::new(),
        metrics: RunMetrics::default(),
    };
    sink.run_finished(0, record).unwrap();

    // The broadcast delta is the lean summary — never the full record.
    let (_, delta) = receiver.try_recv().expect("a delta broadcasts");
    let LiveEvent::RunSummarized { summary, run, .. } = delta else {
        panic!("expected RunSummarized, got {delta:?}");
    };
    assert_eq!(run, 0);
    assert!(
        receiver.try_recv().is_err(),
        "only the summary broadcasts; the full RunCompleted never does"
    );

    // The summary's usages mirror the record's `ModelCalled` events, in order.
    assert_eq!(summary.usages.len(), 2);
    assert_eq!(summary.usages[0].prompt_tokens, Some(100));
    assert_eq!(summary.usages[0].cache_read_tokens, Some(90));
    assert_eq!(summary.usages[1].prompt_tokens, Some(200));

    // The sidecar line is the full `RunCompleted`, carrying the whole event log for resume.
    let (scenario, record) = last_completed(&sidecar);
    assert_eq!(scenario, 0);
    assert_eq!(record.events.len(), 2, "the sidecar keeps the whole log");

    std::fs::remove_dir_all(&dir).ok();
}

/// `run_record` returns a completed run's full record and `None` for a scenario or run that is absent.
#[test]
fn run_record_returns_the_record_or_none() {
    let dir = temp_dir("fetch");
    let sidecar = dir.join("fetch.jsonl");
    let sink = EvalSink::new(meta(), vec![scenario_meta("seam")], &sidecar).expect("sink opens");

    assert!(sink.run_record(0, 0).is_none(), "no run has completed yet");

    let record = RunRecord {
        index: 3,
        started_at_ms: 0,
        finished_at_ms: 0,
        events: vec![model_called(1, usage(50, 40))],
        journal: Vec::new(),
        verdicts: Vec::new(),
        metrics: RunMetrics::default(),
    };
    sink.run_finished(0, record).unwrap();

    let fetched = sink
        .run_record(0, 3)
        .expect("the completed run is fetchable");
    assert_eq!(fetched.index, 3);
    assert_eq!(fetched.events.len(), 1);
    assert!(sink.run_record(0, 7).is_none(), "no such run index");
    assert!(sink.run_record(5, 3).is_none(), "no such scenario index");

    std::fs::remove_dir_all(&dir).ok();
}

/// Read the last `RunCompleted` line the sidecar holds — the authoritative full record a resume folds.
fn last_completed(sidecar: &std::path::Path) -> (u32, RunRecord) {
    let text = std::fs::read_to_string(sidecar).expect("sidecar reads");
    text.lines()
        .rev()
        .find_map(|line| match serde_json::from_str::<LiveEvent>(line) {
            Ok(LiveEvent::RunCompleted {
                scenario, record, ..
            }) => Some((scenario, record)),
            _ => None,
        })
        .expect("a RunCompleted line")
}
