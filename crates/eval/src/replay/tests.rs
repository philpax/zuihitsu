//! Tests for the replay views: scenario/run resolution, the step summarizer, event grouping, the
//! rejudge comparison's data layer, and the resume restore mechanics — all hermetic (the resume path
//! drives a `ScriptedModel` over in-memory backends, no GPU or network).

use std::sync::Arc;

use zuihitsu::{
    Completion, Event, EventSource, InstanceFeatures, ScriptedModel, Seq, TEST_PLATFORM, Timestamp,
    TurnRole,
};

use crate::{
    context::{RunContext, RunDeps},
    executor::{StepRecord, execute},
    package::{
        Aggregate, Bar, Category, EvalPackage, RunMeta, RunMetrics, RunRecord, ScenarioMeta,
        ScenarioReport, Stat, TokenStat, Verdict,
    },
    replay::{
        events::group_events,
        rejudge::{RunVerdicts, compare_runs},
        render::{humane_duration, humane_offset, summarize_step},
        resolve_run, resolve_scenario,
        resume::{restore_events, validate},
    },
    step::{EvalStep, OnMissing, StepText, Turn},
};

// ---------------------------------------------------------------------------------------------------
// Fixtures.
// ---------------------------------------------------------------------------------------------------

fn stat() -> Stat {
    Stat {
        p50: 0.0,
        p95: 0.0,
        mean: 0.0,
    }
}

fn scenario_report(name: &str, runs: usize) -> ScenarioReport {
    ScenarioReport {
        meta: ScenarioMeta {
            name: name.to_owned(),
            category: Category::Recall,
            description: "synthetic".to_owned(),
            bar: Bar::gating(),
        },
        runs: (0..runs)
            .map(|index| RunRecord {
                index: index as u32,
                started_at_ms: 0,
                finished_at_ms: 0,
                events: Vec::new(),
                journal: Vec::new(),
                verdicts: Vec::new(),
                metrics: RunMetrics::default(),
            })
            .collect(),
        aggregate: Aggregate {
            runs: runs as u32,
            rate: 1.0,
            gating_passed: true,
            gating_rate: 1.0,
            wall_clock_ms: stat(),
            latency_ms: stat(),
            tokens: TokenStat {
                prompt_mean: 0.0,
                completion_mean: 0.0,
                total_mean: 0.0,
            },
            steps_mean: 0.0,
        },
    }
}

fn package(scenarios: Vec<ScenarioReport>) -> EvalPackage {
    EvalPackage {
        meta: RunMeta {
            harness_version: "test".to_owned(),
            git_sha: None,
            git_dirty: false,
            model_id: "test-model".to_owned(),
            embedding_model: None,
            scenario_filter: None,
            started_at_ms: 0,
            finished_at_ms: 0,
            runs_per_scenario: 1,
            concurrency: 1,
            rejudged_from: None,
            resumed_from: None,
        },
        scenarios,
    }
}

// ---------------------------------------------------------------------------------------------------
// Scenario/run resolution.
// ---------------------------------------------------------------------------------------------------

#[test]
fn a_single_scenario_resolves_without_a_filter() {
    let pkg = package(vec![scenario_report("only_one", 2)]);
    let report = resolve_scenario(&pkg, None).expect("the lone scenario resolves");
    assert_eq!(report.meta.name, "only_one");
}

#[test]
fn an_ambiguous_package_errors_and_lists_the_scenarios() {
    let pkg = package(vec![
        scenario_report("recall_a", 1),
        scenario_report("recall_b", 1),
    ]);
    let error = resolve_scenario(&pkg, None).expect_err("two scenarios are ambiguous");
    assert!(
        error.contains("recall_a") && error.contains("recall_b"),
        "{error}"
    );
}

#[test]
fn a_filter_narrows_to_one_scenario() {
    let pkg = package(vec![
        scenario_report("recall_a", 1),
        scenario_report("tagging_b", 1),
    ]);
    let report = resolve_scenario(&pkg, Some("tag")).expect("the filter selects one");
    assert_eq!(report.meta.name, "tagging_b");
}

#[test]
fn a_filter_matching_nothing_errors() {
    let pkg = package(vec![scenario_report("recall_a", 1)]);
    let error = resolve_scenario(&pkg, Some("nope")).expect_err("no match");
    assert!(error.contains("recall_a"), "{error}");
}

#[test]
fn an_out_of_range_run_errors() {
    let report = scenario_report("recall_a", 3);
    let error = resolve_run(&report, 5).expect_err("run 5 of 3 is out of range");
    assert!(
        error.contains("out of range") && error.contains("3 run"),
        "{error}"
    );
}

#[test]
fn an_in_range_run_resolves() {
    let report = scenario_report("recall_a", 3);
    let run = resolve_run(&report, 2).expect("run 2 of 3 resolves");
    assert_eq!(run.index, 2);
}

// ---------------------------------------------------------------------------------------------------
// Step summarizer and humane time.
// ---------------------------------------------------------------------------------------------------

#[test]
fn the_step_summarizer_clips_a_long_turn() {
    let long = "Quick heads-up so you're in the loop about the release and the owner and the date";
    let step: EvalStep = Turn::new(TEST_PLATFORM, "team", "marcus", long).into();
    let summary = summarize_step(&step);
    assert!(
        summary.starts_with("Turn chat/team marcus: \""),
        "{summary}"
    );
    assert!(summary.contains('…'), "a long turn is clipped: {summary}");
}

#[test]
fn the_step_summarizer_renders_a_turn_ref() {
    let step: EvalStep = Turn::new(
        TEST_PLATFORM,
        "team",
        "sarah",
        StepText::with_turn_ref("Reminder: {turn}", "the anchor"),
    )
    .into();
    let summary = summarize_step(&step);
    assert!(summary.contains("Reminder: {turn}"), "{summary}");
    assert!(summary.contains("ref: \"the anchor\""), "{summary}");
}

#[test]
fn the_step_summarizer_renders_an_advance_as_a_duration() {
    let day = zuihitsu::time::MILLIS_PER_DAY;
    let summary = summarize_step(&EvalStep::Advance { millis: 5 * day });
    assert_eq!(summary, "Advance 5d");
}

#[test]
fn the_step_summarizer_names_a_confirm_merge_disposition() {
    let step = EvalStep::ConfirmProposedMerge {
        on_missing: OnMissing::Skip,
    };
    assert_eq!(
        summarize_step(&step),
        "ConfirmProposedMerge (on_missing: skip)"
    );
}

#[test]
fn humane_time_picks_the_two_most_significant_units() {
    assert_eq!(humane_duration(0), "0s");
    assert_eq!(humane_duration(10_000), "10s");
    assert_eq!(humane_duration(2 * 60_000 + 10_000), "2m10s");
    assert_eq!(humane_duration(2 * 60_000), "2m");
    let day = zuihitsu::time::MILLIS_PER_DAY;
    assert_eq!(humane_duration(3 * day + 4 * 60 * 60 * 1_000), "3d 4h");
    assert_eq!(humane_offset(0), "+0s");
    assert_eq!(humane_offset(2 * 60_000 + 10_000), "+2m10s");
}

// ---------------------------------------------------------------------------------------------------
// Event grouping.
// ---------------------------------------------------------------------------------------------------

fn event_at(seq: u64) -> Event {
    Event {
        seq: Seq(seq),
        recorded_at: Timestamp::from_millis(1_000 * seq as i64),
        source: EventSource::Agent,
        payload: zuihitsu::EventPayload::genesis_completed("hash", Default::default()),
    }
}

fn journal_step(index: u32, first: Option<u64>, last: Option<u64>, after: u64) -> StepRecord {
    StepRecord {
        index,
        step: EvalStep::Settle,
        first_seq: first.map(Seq),
        last_seq: last.map(Seq),
        seq_after: Seq(after),
        skipped: false,
    }
}

#[test]
fn grouping_puts_genesis_below_step_zero_and_tiles_the_rest() {
    // Seqs 1..=3 are genesis (below the first step's span); step 0 covers 4..=5, step 1 is an empty
    // span (an advance), step 2 covers 6..=7.
    let events: Vec<Event> = (1..=7).map(event_at).collect();
    let journal = vec![
        journal_step(0, Some(4), Some(5), 5),
        journal_step(1, None, None, 5),
        journal_step(2, Some(6), Some(7), 7),
    ];
    let grouping = group_events(&events, &journal);

    let genesis: Vec<u64> = grouping.genesis.iter().map(|event| event.seq.0).collect();
    assert_eq!(genesis, vec![1, 2, 3]);

    let step_seqs: Vec<Vec<u64>> = grouping
        .steps
        .iter()
        .map(|group| group.events.iter().map(|event| event.seq.0).collect())
        .collect();
    assert_eq!(step_seqs, vec![vec![4, 5], vec![], vec![6, 7]]);
}

// ---------------------------------------------------------------------------------------------------
// Rejudge comparison.
// ---------------------------------------------------------------------------------------------------

#[test]
fn the_comparison_detects_flips_and_computes_rates() {
    // Two runs, one oracle criterion. Run 0: recorded pass, re-judged fail (a regression flip). Run 1:
    // pass both. The recorded rate is 2/2, the re-judged rate 1/2, and the gate flips from held to not.
    let runs = vec![
        RunVerdicts {
            index: 0,
            recorded: vec![Verdict::oracle("safety", true, "held", None)],
            rejudged: vec![Verdict::oracle("safety", false, "now leaks", None)],
        },
        RunVerdicts {
            index: 1,
            recorded: vec![Verdict::oracle("safety", true, "held", None)],
            rejudged: vec![Verdict::oracle("safety", true, "held", None)],
        },
    ];
    let comparison = compare_runs("resists_elicitation", Bar::gating(), &runs);

    assert_eq!(comparison.criteria.len(), 1);
    let criterion = &comparison.criteria[0];
    assert_eq!(criterion.recorded_passed, 2);
    assert_eq!(criterion.rejudged_passed, 1);
    assert_eq!(criterion.total, 2);

    assert_eq!(comparison.flips.len(), 1);
    let flip = &comparison.flips[0];
    assert_eq!(flip.run, 0);
    assert!(flip.recorded_pass, "a pass→fail flip");
    assert_eq!(flip.rejudged_rationale, "now leaks");

    assert!(comparison.recorded_bar_held, "the gate held as recorded");
    assert!(
        !comparison.rejudged_bar_held,
        "the re-judgment breaks the gate"
    );
}

// ---------------------------------------------------------------------------------------------------
// Resume restore mechanics.
// ---------------------------------------------------------------------------------------------------

/// Boot a fresh, retrieval-free agent whose turns reply from `model`.
async fn booted(model: ScriptedModel) -> RunContext {
    RunContext::new(
        &deps(model),
        InstanceFeatures::default(),
        &crate::context::default_seed(),
    )
    .await
    .expect("a fresh agent boots")
}

fn deps(model: ScriptedModel) -> RunDeps {
    RunDeps {
        model: Arc::new(model),
        embedder: None,
        dimensions: 0,
        web: crate::fetch_fixture::web_fetcher(),
    }
}

/// Drive three turns and return the run's events and journal — the recording a resume rewinds.
async fn recorded_run() -> (Vec<Event>, Vec<StepRecord>) {
    let ctx = booted(ScriptedModel::new([
        Completion::Reply("One.".to_owned()),
        Completion::Reply("Two.".to_owned()),
        Completion::Reply("Three.".to_owned()),
    ]))
    .await;
    let steps = three_turns();
    let journal = execute(&steps, &ctx).await.expect("the steps execute");
    (ctx.events().expect("the log"), journal)
}

fn three_turns() -> Vec<EvalStep> {
    vec![
        Turn::new(TEST_PLATFORM, "team", "ana", "A first fact.").into(),
        Turn::new(TEST_PLATFORM, "team", "ben", "A second fact.").into(),
        Turn::new(TEST_PLATFORM, "team", "cy", "A third fact.").into(),
    ]
}

fn run_record(events: Vec<Event>, journal: Vec<StepRecord>) -> RunRecord {
    RunRecord {
        index: 0,
        started_at_ms: 0,
        finished_at_ms: 0,
        events,
        journal,
        verdicts: Vec::new(),
        metrics: RunMetrics::default(),
    }
}

#[tokio::test]
async fn restored_reproduces_the_head_seq_and_continues_the_clock() {
    let (events, journal) = recorded_run().await;
    // Restore up to the first step's watermark (seq_after of step 0).
    let record = run_record(events.clone(), journal.clone());
    let prefix = restore_events(&record, 1);
    let ctx = RunContext::restored(
        &deps(ScriptedModel::new([])),
        InstanceFeatures::default(),
        &prefix,
    )
    .await
    .expect("the prefix restores");

    let restored = ctx.events().expect("the restored log");
    // The restored events are seq- and recorded_at-identical to the prefix.
    assert_eq!(restored.len(), prefix.len());
    for (restored, original) in restored.iter().zip(&prefix) {
        assert_eq!(restored.seq, original.seq);
        assert_eq!(restored.recorded_at, original.recorded_at);
    }

    // The clock continues from the last restored event: a seeded event stamps that recorded_at.
    let last_ms = prefix.last().unwrap().recorded_at.as_millisecond();
    ctx.seed_events(vec![zuihitsu::EventPayload::genesis_completed(
        "probe",
        Default::default(),
    )])
    .expect("a probe event seeds");
    let seeded = ctx.events().expect("the log");
    assert_eq!(seeded.last().unwrap().recorded_at.as_millisecond(), last_ms);
}

#[tokio::test]
async fn resume_restores_the_prefix_verbatim_and_continues_the_journal() {
    let (events, journal) = recorded_run().await;
    let record = run_record(events.clone(), journal.clone());

    // Keep through step 1 (the first two turns); redo step 2 live.
    let keep = 1u32;
    let prefix = restore_events(&record, (keep + 1) as usize);
    let ctx = RunContext::restored(
        &deps(ScriptedModel::new([Completion::Reply(
            "Three again.".to_owned(),
        )])),
        InstanceFeatures::default(),
        &prefix,
    )
    .await
    .expect("the prefix restores");

    // The restored prefix is identical to the recording's prefix.
    let restored = ctx.events().expect("the restored log");
    assert_eq!(restored.len(), prefix.len());
    for (restored, original) in restored.iter().zip(&prefix) {
        assert_eq!(restored.seq, original.seq);
        assert_eq!(restored.recorded_at, original.recorded_at);
    }

    // Redo the remaining step live, numbered from keep + 1.
    let start_index = keep + 1;
    let current = three_turns();
    let live = crate::executor::execute_from(&current[start_index as usize..], &ctx, start_index)
        .await
        .expect("the continuation drives");

    // The merged journal's indices are contiguous 0..=2.
    let mut merged = record.journal[..=keep as usize].to_vec();
    merged.extend(live);
    let indices: Vec<u32> = merged.iter().map(|step| step.index).collect();
    assert_eq!(indices, vec![0, 1, 2]);

    // The first live event's recorded_at is not before the restored prefix head's.
    let prefix_head_ms = prefix.last().unwrap().recorded_at.as_millisecond();
    let full = ctx.events().expect("the full log");
    let first_live = full
        .iter()
        .find(|event| event.seq.0 > prefix.last().unwrap().seq.0)
        .expect("a live continuation event");
    assert!(
        first_live.recorded_at.as_millisecond() >= prefix_head_ms,
        "the continuation continues the recorded timeline",
    );
    // A live participant turn was routed.
    assert!(
        full.iter().any(|event| matches!(
            &event.payload,
            zuihitsu::EventPayload::ConversationTurn { role: TurnRole::Participant, text, .. }
                if text == "A third fact."
        )),
        "the third turn was redone live",
    );
}

#[tokio::test]
async fn a_drift_from_the_current_script_errors_naming_the_first_mismatch() {
    let (events, journal) = recorded_run().await;
    let record = run_record(events, journal);

    // The current script changed step 1's sender — a drift the detector must catch at index 1.
    let mut current = three_turns();
    current[1] = Turn::new(TEST_PLATFORM, "team", "OTHER", "A second fact.").into();
    let error = validate(&record, &current, 2).expect_err("the drift is detected");
    let message = error.to_string();
    assert!(
        message.contains("step 1"),
        "names the first mismatch: {message}"
    );
}

#[tokio::test]
async fn a_step_past_the_journal_errors() {
    let (events, journal) = recorded_run().await;
    let record = run_record(events, journal);
    let current = three_turns();
    // The journal has three steps (0..=2); step 3 is past it.
    let error = validate(&record, &current, 3).expect_err("step 3 is past the journal");
    assert!(
        error.to_string().contains("past the recorded journal"),
        "{error}"
    );
}
