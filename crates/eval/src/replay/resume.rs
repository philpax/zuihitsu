//! The `replay --mode resume` path: rewind one recorded run to a chosen journal step, restore its log
//! verbatim up to that step's watermark, and redo the rest of the scenario live from that point against
//! the current code and model. The restored prefix is exactly the state the original run held at step K,
//! so the continuation runs against that state rather than a re-driven approximation of it. Never writes
//! trend history.

use std::{path::Path, sync::Arc, time::Instant};

use zuihitsu::{Embedder, EnvConfig, Event, ModelClient, OpenAiClient, OpenAiEmbedder};

use crate::{
    context::{RunContext, RunDeps},
    error::EvalError,
    executor::{StepRecord, execute_from},
    fetch_fixture, harness,
    judge::Judge,
    package::{
        Aggregate, EvalPackage, ResumeProvenance, RunMeta, RunRecord, ScenarioReport, VerdictKind,
    },
    replay::{render::summarize_step, resolve_run, resolve_scenario},
    retry::{RetryingEmbedder, RetryingModel},
    scenario::Scenario,
};

/// Resume `package`'s scenario/run from journal step `step`: validate the recording against the current
/// script, restore the prefix, redo the rest live, and write the result to `eval/<name>.json` (a derived
/// name when none is given). `scenario`, `run`, and `step` are all required (validated by the caller).
pub(crate) async fn resume(
    package: &Path,
    scenario: &str,
    run: usize,
    step: u32,
    config_path: &Path,
    name: Option<&str>,
) -> Result<(), EvalError> {
    let pkg = crate::replay::load(package)?;
    let report = resolve_scenario(&pkg, Some(scenario)).map_err(EvalError::Replay)?;
    let record = resolve_run(report, run).map_err(EvalError::Replay)?;

    let registry = crate::scenarios::all();
    let scenario_impl = registry
        .iter()
        .find(|candidate| candidate.meta().name == report.meta.name)
        .ok_or_else(|| {
            EvalError::Replay(format!(
                "scenario {:?} is not in the current registry, so it cannot be resumed",
                report.meta.name
            ))
        })?;
    let current_steps = scenario_impl.steps();

    let prefix = validate(record, &current_steps, step)?;
    let restored = restore_events(record, prefix);

    let deps = build_deps(config_path, scenario_impl.as_ref())?;
    let judge = Judge::new(deps.model.clone());
    let features = scenario_impl.features();
    let ctx = RunContext::restored(&deps, features, &restored).await?;
    if scenario_impl.needs_retrieval() {
        // The recorded timeline indexed incrementally; catch the restored content up so it is searchable
        // before the first live step, as it was in the original run.
        ctx.index_catch_up().await?;
    }

    let start_index = step + 1;
    let continuation = &current_steps[start_index as usize..];
    let started_at_ms = crate::live::now_ms();
    let started = Instant::now();
    let live_journal = execute_from(continuation, &ctx, start_index).await?;
    let wall_clock_ms = started.elapsed().as_millis() as u64;

    let events = ctx.events()?;
    let verdicts = scenario_impl.assess(&events, &judge).await;
    let gating_passed = verdicts
        .iter()
        .filter(|verdict| matches!(verdict.kind, VerdictKind::Oracle))
        .all(|verdict| verdict.passed);
    let metrics = harness::run_metrics(&events, gating_passed, wall_clock_ms);

    // The merged journal: the recorded prefix (0..=step) followed by the live continuation, so the step
    // indices stay contiguous.
    let mut journal: Vec<StepRecord> = record.journal[..=step as usize].to_vec();
    journal.extend(live_journal);

    let result = assemble_package(
        &pkg,
        report,
        run,
        step,
        package,
        RunRecord {
            index: 0,
            started_at_ms,
            finished_at_ms: crate::live::now_ms(),
            events,
            journal,
            verdicts,
            metrics,
        },
    );

    let default_name = format!("{}-resume-r{run}-s{step}", source_stem(package));
    let name = name.unwrap_or(&default_name);
    let out = Path::new(crate::run::EVAL_DIR).join(format!("{name}.json"));
    crate::run::write_package(&result, &out)?;

    let report = &result.scenarios[0];
    let held = report.runs[0].verdicts.iter().filter(|v| v.passed).count();
    let total = report.runs[0].verdicts.len();
    let gate = if report.aggregate.gating_passed {
        "gate ok"
    } else {
        "gate FAIL"
    };
    println!(
        "resumed {} run {} from step {}; redid {} step(s) live",
        report.meta.name,
        run,
        step,
        continuation.len(),
    );
    println!(
        "wrote {} — {held}/{total} verdicts held, {gate}",
        out.display()
    );
    Ok(())
}

/// Validate the recording against the current script and return the number of prefix steps to keep
/// (`step + 1`). The run must have a journal, `step` must be within it, the current script's steps
/// `0..=step` must agree structurally with the recorded ones (the drift detector), and the current
/// script must have steps past `step` to redo.
pub(super) fn validate(
    record: &RunRecord,
    current_steps: &[crate::step::EvalStep],
    step: u32,
) -> Result<usize, EvalError> {
    if record.journal.is_empty() {
        return Err(EvalError::Replay(
            "this run was recorded before step journaling, so it can only be rejudged, not resumed"
                .to_owned(),
        ));
    }
    let step = step as usize;
    if step >= record.journal.len() {
        return Err(EvalError::Replay(format!(
            "step {step} is past the recorded journal, which has {} step(s) (0..={})",
            record.journal.len(),
            record.journal.len() - 1,
        )));
    }
    if step + 1 > current_steps.len() {
        return Err(EvalError::Replay(format!(
            "the current script has only {} step(s), too few to keep through step {step}",
            current_steps.len(),
        )));
    }
    for (index, (recorded, current)) in record.journal[..=step]
        .iter()
        .zip(&current_steps[..=step])
        .enumerate()
    {
        if &recorded.step != current {
            return Err(EvalError::Replay(format!(
                "the recorded run drifted from the current script at step {index}:\n  recorded: \
                 {}\n  current:  {}",
                summarize_step(&recorded.step),
                summarize_step(current),
            )));
        }
    }
    if current_steps.len() <= step + 1 {
        return Err(EvalError::Replay(format!(
            "the current script ends at step {step}, so there is nothing past it to redo live"
        )));
    }
    Ok(step + 1)
}

/// The recorded events to restore: the log up to and including the kept prefix's watermark (the
/// `seq_after` of step `prefix - 1`), genesis included.
pub(super) fn restore_events(record: &RunRecord, prefix: usize) -> Vec<Event> {
    let watermark = record.journal[prefix - 1].seq_after;
    record
        .events
        .iter()
        .filter(|event| event.seq <= watermark)
        .cloned()
        .collect()
}

/// Build the run deps from `config_path` the way `run` does — the model (retrying), the embedder and
/// its dimensionality when an embedding endpoint is configured, and the test fetch MCP host. A scenario
/// that needs retrieval but has no embedder configured is an error here (where `run` would silently skip
/// it), since resume targets exactly this scenario.
fn build_deps(config_path: &Path, scenario: &dyn Scenario) -> Result<RunDeps, EvalError> {
    let config = EnvConfig::load(config_path).map_err(|source| EvalError::LoadConfig {
        path: config_path.to_path_buf(),
        source: Box::new(source),
    })?;
    if config.model.endpoint.is_empty() {
        return Err(EvalError::Replay(
            "resume needs a model endpoint to drive the continuation; none is configured"
                .to_owned(),
        ));
    }
    let model: Arc<dyn ModelClient> = Arc::new(RetryingModel::new(Arc::new(OpenAiClient::new(
        &config.model,
    ))));
    let embedder: Option<Arc<dyn Embedder>> = (!config.embedding.endpoint.is_empty()).then(|| {
        Arc::new(RetryingEmbedder::new(Arc::new(OpenAiEmbedder::new(
            &config.embedding,
        )))) as Arc<dyn Embedder>
    });
    if scenario.needs_retrieval() && embedder.is_none() {
        return Err(EvalError::Replay(format!(
            "scenario {:?} needs retrieval, but no embedding endpoint is configured",
            scenario.meta().name
        )));
    }
    Ok(RunDeps {
        model,
        embedder,
        dimensions: config.embedding.dimensions,
        web: fetch_fixture::web_fetcher(),
    })
}

/// Assemble the one-scenario, one-run result package: the scenario meta carried from the source, the
/// resumed run, its recomputed aggregate, and the meta stamped with resume provenance. Never history.
fn assemble_package(
    source: &EvalPackage,
    report: &ScenarioReport,
    run: usize,
    step: u32,
    package: &Path,
    resumed: RunRecord,
) -> EvalPackage {
    let aggregate = single_run_aggregate(&resumed);
    let started_at_ms = crate::live::now_ms();
    EvalPackage {
        meta: RunMeta {
            harness_version: env!("CARGO_PKG_VERSION").to_owned(),
            git_sha: crate::run::git_sha(),
            git_dirty: crate::run::git_dirty(),
            model_id: source.meta.model_id.clone(),
            embedding_model: source.meta.embedding_model.clone(),
            scenario_filter: Some(report.meta.name.clone()),
            started_at_ms,
            finished_at_ms: started_at_ms,
            runs_per_scenario: 1,
            concurrency: 1,
            rejudged_from: None,
            resumed_from: Some(ResumeProvenance {
                package: package.display().to_string(),
                scenario: report.meta.name.clone(),
                run: run as u32,
                step,
            }),
        },
        scenarios: vec![ScenarioReport {
            meta: report.meta.clone(),
            runs: vec![resumed],
            aggregate,
        }],
    }
}

/// The aggregate over a single resumed run — the same computation `run` uses, over the one run.
fn single_run_aggregate(run: &RunRecord) -> Aggregate {
    harness::aggregate(std::slice::from_ref(run))
}

fn source_stem(package: &Path) -> String {
    package
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "package".to_owned())
}
