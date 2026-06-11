//! The runner, in two phases so assessment never contends with the evals for the model. Phase one
//! drives every run (agent turns only) and collects each run's event log; phase two judges those
//! collected logs (judge calls only). Both phases are bounded by the same concurrency. Then metrics
//! from the `ModelCalled` events and an aggregate per scenario.

use std::{collections::BTreeMap, sync::Arc};

use tokio::{sync::Semaphore, task::JoinSet};
use zuihitsu::{Event, EventPayload, GenerateRequest, Message, ModelPhase};

use crate::{
    context::{RunContext, RunDeps},
    judge::Judge,
    package::{
        Aggregate, RunMetrics, RunRecord, ScenarioReport, Stat, TokenStat, Verdict, VerdictKind,
    },
    scenario::Scenario,
};

/// One run's log after phase one: the event log, or the reason it did not complete.
type DrivenRun = Result<Vec<Event>, String>;

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

/// Run every scenario `runs` times at most `concurrency` at a time, returning the per-scenario reports
/// in registry order. A scenario needing retrieval is skipped (with a warning) when `deps.embedder` is
/// `None`.
pub async fn run_all(
    deps: RunDeps,
    scenarios: Vec<Arc<dyn Scenario>>,
    runs: u32,
    concurrency: usize,
) -> Vec<ScenarioReport> {
    let permits = Arc::new(Semaphore::new(concurrency.max(1)));
    let has_retrieval = deps.embedder.is_some();

    // The scenarios that will actually run, in registry order.
    let mut active: Vec<(usize, Arc<dyn Scenario>)> = Vec::new();
    for (index, scenario) in scenarios.iter().enumerate() {
        if scenario.needs_retrieval() && !has_retrieval {
            tracing::warn!(scenario = %scenario.meta().name, "skipping: needs retrieval, but no embedding endpoint is configured");
            continue;
        }
        active.push((index, scenario.clone()));
    }

    // Phase one: drive every run, collecting its event log. No judging here — the model serves only
    // the agent's own turns.
    let logs = drive_phase(&active, &deps, runs, &permits).await;
    tracing::info!("phase one complete: all runs driven; assessing");

    // Phase two: judge the collected logs. The model now serves only the judge.
    assess_phase(active, logs, &deps, &permits).await
}

async fn drive_phase(
    active: &[(usize, Arc<dyn Scenario>)],
    deps: &RunDeps,
    runs: u32,
    permits: &Arc<Semaphore>,
) -> BTreeMap<usize, Vec<(u32, DrivenRun)>> {
    let mut set: JoinSet<(usize, u32, DrivenRun)> = JoinSet::new();
    for (index, scenario) in active {
        for run_index in 0..runs {
            // Acquire before spawning so at most `concurrency` runs are ever in flight.
            let permit = permits
                .clone()
                .acquire_owned()
                .await
                .expect("semaphore open");
            let scenario = scenario.clone();
            let deps = deps.clone();
            let index = *index;
            set.spawn(async move {
                let _permit = permit;
                let driven = drive(scenario.as_ref(), &deps)
                    .await
                    .map_err(|e| e.to_string());
                (index, run_index, driven)
            });
        }
    }
    let mut logs: BTreeMap<usize, Vec<(u32, DrivenRun)>> = BTreeMap::new();
    while let Some(joined) = set.join_next().await {
        let (index, run_index, driven) = joined.expect("an eval drive task panicked");
        logs.entry(index).or_default().push((run_index, driven));
    }
    logs
}

async fn assess_phase(
    active: Vec<(usize, Arc<dyn Scenario>)>,
    mut logs: BTreeMap<usize, Vec<(u32, DrivenRun)>>,
    deps: &RunDeps,
    permits: &Arc<Semaphore>,
) -> Vec<ScenarioReport> {
    let judge = Arc::new(Judge::new(deps.model.clone()));
    let mut set: JoinSet<(usize, RunRecord)> = JoinSet::new();
    for (index, scenario) in &active {
        for (run_index, driven) in logs.remove(index).unwrap_or_default() {
            let permit = permits
                .clone()
                .acquire_owned()
                .await
                .expect("semaphore open");
            let scenario = scenario.clone();
            let judge = judge.clone();
            let index = *index;
            set.spawn(async move {
                let _permit = permit;
                (
                    index,
                    assess(scenario.as_ref(), run_index, driven, &judge).await,
                )
            });
        }
    }
    let mut by_scenario: BTreeMap<usize, Vec<RunRecord>> = BTreeMap::new();
    while let Some(joined) = set.join_next().await {
        let (index, record) = joined.expect("an eval assess task panicked");
        by_scenario.entry(index).or_default().push(record);
    }

    let mut reports = Vec::new();
    for (index, scenario) in active {
        let mut runs = by_scenario.remove(&index).unwrap_or_default();
        runs.sort_by_key(|run| run.index);
        let aggregate = aggregate(&runs);
        reports.push(ScenarioReport {
            meta: scenario.meta(),
            runs,
            aggregate,
        });
    }
    reports
}

/// Drive one run: a fresh agent, the scenario's turns, then its event log.
async fn drive(
    scenario: &dyn Scenario,
    deps: &RunDeps,
) -> Result<Vec<Event>, crate::error::EvalError> {
    let ctx = RunContext::new(deps).await?;
    scenario.run(&ctx).await?;
    ctx.events()
}

/// Judge one run's collected log into a record (or, for a run that did not complete, a visible failure).
async fn assess(
    scenario: &dyn Scenario,
    index: u32,
    driven: DrivenRun,
    judge: &Judge,
) -> RunRecord {
    match driven {
        Ok(events) => {
            let verdicts = scenario.assess(&events, judge).await;
            let gating_passed = verdicts
                .iter()
                .filter(|verdict| matches!(verdict.kind, VerdictKind::Oracle))
                .all(|verdict| verdict.passed);
            let metrics = run_metrics(&events, gating_passed);
            RunRecord {
                index,
                events,
                verdicts,
                metrics,
            }
        }
        // An infrastructure failure (the model dropped, say) is visible and lowers the rate, but it is
        // a metric, not a gating leak — a flake must not be reported as a safety regression.
        Err(error) => RunRecord {
            index,
            events: Vec::new(),
            verdicts: vec![Verdict::metric(
                "the run completed",
                false,
                format!("the run did not complete: {error}"),
            )],
            metrics: RunMetrics {
                gating_passed: true,
                ..RunMetrics::default()
            },
        },
    }
}

/// Sum the run's `ModelCalled` events into its metrics.
fn run_metrics(events: &[Event], gating_passed: bool) -> RunMetrics {
    let mut metrics = RunMetrics {
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
/// invariant (no oracle regressed in any run), and the latency/token/step distributions.
fn aggregate(runs: &[RunRecord]) -> Aggregate {
    let n = runs.len().max(1) as f64;
    let passed = runs
        .iter()
        .filter(|run| run.verdicts.iter().all(|verdict| verdict.passed))
        .count();
    let gating_passed = runs.iter().all(|run| run.metrics.gating_passed);

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
fn percentile(values: &[f64], q: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let rank = (q * (sorted.len() as f64 - 1.0)).round() as usize;
    sorted[rank.min(sorted.len() - 1)]
}
