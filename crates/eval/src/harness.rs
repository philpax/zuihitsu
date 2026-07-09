//! The runner: every scenario `runs` times, each run a `drive → assess` pipeline emitted through the
//! [`EvalSink`] as it completes, all bounded by one concurrency limit. A run's metrics come from its own
//! `ModelCalled` events; the judge's calls go through a separate client and never enter the run's log,
//! so interleaving drive and assess leaves per-run metrics intact. The sink folds each completed run
//! into the growing package and the live log.

use std::{
    collections::HashSet,
    sync::Arc,
    time::{Duration, Instant},
};

use tokio::{sync::Semaphore, task::JoinSet};
use zuihitsu::{Event, EventPayload, GenerateRequest, Message, ModelPhase, Seq};

/// How often a driving run's log is polled for new events to stream live. Short enough that a
/// deliberation reads as unfolding, and the read is incremental, so it costs only the new events.
const POLL_INTERVAL: Duration = Duration::from_millis(200);

use crate::{
    context::{RunContext, RunDeps},
    error::EvalError,
    executor::{StepRecord, execute},
    judge::Judge,
    live::{EvalSink, now_ms},
    package::{Aggregate, RunMetrics, RunRecord, Stat, TokenStat, Verdict, VerdictKind},
    scenario::Scenario,
};

/// One run after phase one: its event log and the executor's per-step journal (or the reason it did
/// not complete), and the wall-clock it took to drive — the truthful cost, since it includes the
/// synchronous catch-ups whose synthesis records no `ModelCalled` (spec §Write path).
struct DrivenRun {
    outcome: Result<(Vec<Event>, Vec<StepRecord>), String>,
    wall_clock_ms: u64,
}

/// Issue one throwaway call to each model in play before any run is timed, so the serving layer has
/// loaded its weights. Otherwise the first run absorbs cold-start latency that is the endpoint's, not
/// the agent's. A warm-up failure is logged and ignored — the runs themselves surface a dead endpoint.
pub async fn warm_up(deps: &RunDeps) {
    let request = GenerateRequest {
        system: "You are warming up.".to_owned(),
        messages: vec![Message::user("Reply with the single word: ok.")],
        ..GenerateRequest::default()
    };
    if let Err(error) = deps.model.generate(&request).await {
        tracing::warn!(%error, "the model warm-up call failed; proceeding");
    }
    if let Some(embedder) = &deps.embedder
        && let Err(error) = embedder.embed(&["warm up".to_owned()]).await
    {
        tracing::warn!(%error, "the embedder warm-up call failed; proceeding");
    }
}

/// The scenarios that will actually run, in registry order — every scenario, minus those needing
/// retrieval when no embedding endpoint is configured, and minus those needing MCP when no test host
/// is configured (skipped with a warning). The manifest and the `scenario` indices in the live log
/// are built over this list, so its order is the scoreboard's order.
pub fn active_scenarios(
    scenarios: Vec<Arc<dyn Scenario>>,
    has_retrieval: bool,
    has_mcp: bool,
) -> Vec<Arc<dyn Scenario>> {
    scenarios
        .into_iter()
        .filter(|scenario| {
            let keep_retrieval = !scenario.needs_retrieval() || has_retrieval;
            if !keep_retrieval {
                tracing::warn!(scenario = %scenario.meta().name, "skipping: needs retrieval, but no embedding endpoint is configured");
            }
            let keep_mcp = !scenario.needs_mcp() || has_mcp;
            if !keep_mcp {
                tracing::warn!(scenario = %scenario.meta().name, "skipping: needs MCP, but no test MCP host is configured");
            }
            keep_retrieval && keep_mcp
        })
        .collect()
}

/// Run every `active` scenario `runs` times, at most `concurrency` in flight, each run a
/// `drive → assess` pipeline emitted through `sink` as it completes — so a run is wholly done (or not)
/// when it lands, the scoreboard fills in live, and an interrupted run is resumable. `scenario` is the
/// index into `active`. The first sink error aborts the run.
pub async fn run_all(
    deps: RunDeps,
    active: Vec<Arc<dyn Scenario>>,
    runs: u32,
    concurrency: usize,
    sink: Arc<EvalSink>,
    done: HashSet<(u32, u32)>,
) -> Result<(), EvalError> {
    let permits = Arc::new(Semaphore::new(concurrency.max(1)));
    let judge = Arc::new(Judge::new(deps.model.clone()));

    let mut set: JoinSet<Result<(), EvalError>> = JoinSet::new();
    for (scenario_index, scenario) in active.iter().enumerate() {
        for run_index in 0..runs {
            // Skip a run a resumed sidecar already holds — only the missing runs drive.
            if done.contains(&(scenario_index as u32, run_index)) {
                continue;
            }
            // Acquire before spawning so at most `concurrency` runs are ever in flight.
            let permit = permits
                .clone()
                .acquire_owned()
                .await
                .expect("semaphore open");
            let scenario = scenario.clone();
            let deps = deps.clone();
            let judge = judge.clone();
            let sink = sink.clone();
            let scenario_index = scenario_index as u32;
            set.spawn(async move {
                let _permit = permit;
                // The harness's real wall-clock for the viewer's elapsed/projection, and a monotonic
                // `Instant` for the drive-cost metric — two clocks, two purposes.
                let started_at_ms = now_ms();
                sink.run_started(scenario_index, run_index, started_at_ms)?;
                let started = Instant::now();
                let outcome =
                    drive_streaming(scenario.as_ref(), &deps, &sink, scenario_index, run_index)
                        .await?;
                let driven = DrivenRun {
                    outcome,
                    wall_clock_ms: started.elapsed().as_millis() as u64,
                };
                let mut record = assess(scenario.as_ref(), run_index, driven, &judge).await;
                record.started_at_ms = started_at_ms;
                record.finished_at_ms = now_ms();
                sink.run_finished(scenario_index, record)
            });
        }
    }
    while let Some(joined) = set.join_next().await {
        joined.expect("an eval run task panicked")?;
    }
    Ok(())
}

/// Drive one run while streaming its events live: between turns, poll the run's log and emit each new
/// event as a `RunEvent`, so a viewer watches the deliberation unfold rather than seeing it appear all
/// at once on completion. The inner result is the run's full log and the executor's journal (or the
/// reason it did not complete, a metric); the outer result is a sink/IO failure, which aborts the whole
/// run.
async fn drive_streaming(
    scenario: &dyn Scenario,
    deps: &RunDeps,
    sink: &EvalSink,
    scenario_index: u32,
    run_index: u32,
) -> Result<Result<(Vec<Event>, Vec<StepRecord>), String>, EvalError> {
    let features = scenario.features();
    let ctx = match RunContext::new(deps, features).await {
        Ok(ctx) => ctx,
        Err(error) => return Ok(Err(error.to_string())),
    };
    let steps = scenario.steps();
    let run = execute(&steps, &ctx);
    tokio::pin!(run);
    let mut cursor = Seq::ZERO;
    let outcome = loop {
        tokio::select! {
            // Bias toward the run: when a turn completes, finish it before polling again.
            biased;
            result = &mut run => break result,
            _ = tokio::time::sleep(POLL_INTERVAL) => {
                cursor = stream_new_events(&ctx, sink, scenario_index, run_index, cursor)?;
            }
        }
    };
    // Drain whatever was recorded between the last poll and completion (a final turn, the synthesis
    // catch-ups) before the run is judged.
    stream_new_events(&ctx, sink, scenario_index, run_index, cursor)?;
    match outcome {
        Ok(journal) => match ctx.events() {
            Ok(events) => Ok(Ok((events, journal))),
            Err(error) => Ok(Err(error.to_string())),
        },
        Err(error) => Ok(Err(error.to_string())),
    }
}

/// Emit every event recorded at or after `cursor` as a `RunEvent`, returning the cursor past the last —
/// an incremental read, so each poll touches only what is new.
fn stream_new_events(
    ctx: &RunContext,
    sink: &EvalSink,
    scenario_index: u32,
    run_index: u32,
    cursor: Seq,
) -> Result<Seq, EvalError> {
    let mut next = cursor;
    for event in ctx.events_from(cursor)? {
        next = event.seq.next();
        sink.run_event(scenario_index, run_index, event)?;
    }
    Ok(next)
}

/// Judge one run's collected log into a record (or, for a run that did not complete, a visible failure).
async fn assess(
    scenario: &dyn Scenario,
    index: u32,
    driven: DrivenRun,
    judge: &Judge,
) -> RunRecord {
    let DrivenRun {
        outcome,
        wall_clock_ms,
    } = driven;
    match outcome {
        Ok((events, journal)) => {
            let verdicts = scenario.assess(&events, judge).await;
            let gating_passed = verdicts
                .iter()
                .filter(|verdict| matches!(verdict.kind, VerdictKind::Oracle))
                .all(|verdict| verdict.passed);
            let metrics = run_metrics(&events, gating_passed, wall_clock_ms);
            // The wall-clock stamps are set by the caller, which holds the harness's real clock.
            RunRecord {
                index,
                started_at_ms: 0,
                finished_at_ms: 0,
                events,
                journal,
                verdicts,
                metrics,
            }
        }
        // An infrastructure failure (the model dropped, say) is visible and lowers the rate, but it is
        // a metric, not a gating leak — a flake must not be reported as a safety regression.
        Err(error) => RunRecord {
            index,
            started_at_ms: 0,
            finished_at_ms: 0,
            events: Vec::new(),
            journal: Vec::new(),
            verdicts: vec![Verdict::metric(
                "the run completed",
                false,
                format!("the run did not complete: {error}"),
            )],
            metrics: RunMetrics {
                gating_passed: true,
                wall_clock_ms,
                ..RunMetrics::default()
            },
        },
    }
}

/// Sum the run's `ModelCalled` events into its metrics; `wall_clock_ms` is the measured drive time.
fn run_metrics(events: &[Event], gating_passed: bool, wall_clock_ms: u64) -> RunMetrics {
    let mut metrics = RunMetrics {
        wall_clock_ms,
        gating_passed,
        ..RunMetrics::default()
    };
    for event in events {
        if let EventPayload::ModelCalled {
            phase,
            usage,
            duration_ms,
            ..
        } = &event.payload
        {
            metrics.model_calls += 1;
            if *phase == ModelPhase::Step {
                metrics.steps += 1;
            }
            metrics.total_latency_ms += duration_ms;
            metrics.prompt_tokens += usage.prompt_tokens.unwrap_or(0);
            metrics.completion_tokens += usage.completion_tokens.unwrap_or(0);
            metrics.total_tokens += usage.total_tokens.unwrap_or(0);
        }
    }
    metrics
}

/// Aggregate a scenario's runs: the pass rate (a run passes when every verdict passed), the gating
/// invariant (no oracle regressed in any run), and the latency/token/step distributions. Recomputed by
/// the sink after each completed run, so the scoreboard's aggregate is always current.
pub(crate) fn aggregate(runs: &[RunRecord]) -> Aggregate {
    let n = runs.len().max(1) as f64;
    let passed = runs
        .iter()
        .filter(|run| run.verdicts.iter().all(|verdict| verdict.passed))
        .count();
    let gating_passed = runs.iter().all(|run| run.metrics.gating_passed);
    let gating_held = runs.iter().filter(|run| run.metrics.gating_passed).count();
    let gating_rate = gating_held as f64 / n;

    let wall_clocks: Vec<f64> = runs
        .iter()
        .map(|run| run.metrics.wall_clock_ms as f64)
        .collect();
    let latencies: Vec<f64> = runs
        .iter()
        .map(|run| run.metrics.total_latency_ms as f64)
        .collect();
    let steps: Vec<f64> = runs.iter().map(|run| run.metrics.steps as f64).collect();
    let prompt: Vec<f64> = runs
        .iter()
        .map(|run| run.metrics.prompt_tokens as f64)
        .collect();
    let completion: Vec<f64> = runs
        .iter()
        .map(|run| run.metrics.completion_tokens as f64)
        .collect();
    let total: Vec<f64> = runs
        .iter()
        .map(|run| run.metrics.total_tokens as f64)
        .collect();

    Aggregate {
        runs: runs.len() as u32,
        rate: passed as f64 / n,
        gating_passed,
        gating_rate,
        wall_clock_ms: stat(&wall_clocks),
        latency_ms: stat(&latencies),
        tokens: TokenStat {
            prompt_mean: mean(&prompt),
            completion_mean: mean(&completion),
            total_mean: mean(&total),
        },
        steps_mean: mean(&steps),
    }
}

fn stat(values: &[f64]) -> Stat {
    Stat {
        p50: percentile(values, 0.50),
        p95: percentile(values, 0.95),
        mean: mean(values),
    }
}

fn mean(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.iter().sum::<f64>() / values.len() as f64
}

/// The `q`-quantile by nearest-rank over a copy sorted ascending. Small N, so an exact sort is fine.
pub(crate) fn percentile(values: &[f64], q: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let rank = (q * (sorted.len() as f64 - 1.0)).round() as usize;
    sorted[rank.min(sorted.len() - 1)]
}
