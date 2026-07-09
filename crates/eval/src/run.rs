//! The `run` command: drives the scenario suite, writes the package, and serves the live view.

use std::{net::SocketAddr, path::Path, sync::Arc};

use zuihitsu::{Embedder, EnvConfig, ModelClient, OpenAiClient, OpenAiEmbedder};

use crate::{
    context::RunDeps,
    error::EvalError,
    fetch_fixture, live,
    package::{EvalPackage, RunMeta, ScenarioMeta},
    retry::{RetryingEmbedder, RetryingModel},
};

/// The directory every eval run is filed under — kept together so runs are findable rather than
/// scattered to arbitrary paths (or `/tmp`, where they are lost to GC). Gitignored; only the small
/// `history.jsonl` trend record is tracked.
pub(crate) const EVAL_DIR: &str = "eval";

/// The address live serving binds when no override is given.
pub(crate) const DEFAULT_SERVE_ADDR: &str = "127.0.0.1:7878";

/// How a run serves its live view.
pub(crate) struct ServeConfig {
    /// Where to bind the SSE endpoint, or `None` to not serve at all (`--no-serve`).
    pub(crate) addr: Option<SocketAddr>,
    /// Keep serving the final state after the run completes (until Ctrl-C) rather than exiting.
    pub(crate) after_completion: bool,
}

/// Resolve the live-serving config from the flags. Serving is on by default — `--no-serve` opts out,
/// and an explicit `--serve` overrides the bind address — and stops when the run finishes unless
/// `--serve-after-completion` keeps it up for review.
pub(crate) fn resolve_serve(
    serve: Option<SocketAddr>,
    no_serve: bool,
    after_completion: bool,
) -> ServeConfig {
    let addr = (!no_serve).then(|| {
        serve.unwrap_or_else(|| {
            DEFAULT_SERVE_ADDR
                .parse()
                .expect("a valid default serve address")
        })
    });
    ServeConfig {
        addr,
        after_completion,
    }
}

/// Resolve a run `name` to `eval/<name>.json`, rejecting anything that is not a bare filename (so a
/// run cannot escape the eval directory). Then run the suite under it.
pub(crate) async fn run_named(
    runs: u32,
    concurrency: usize,
    filter: Option<&str>,
    name: &str,
    config_path: &Path,
    resume: bool,
    serve: ServeConfig,
) -> Result<bool, EvalError> {
    if name.is_empty()
        || name.contains('/')
        || name.contains('\\')
        || name.split('/').any(|part| part == "..")
        || name == ".."
        || name == "."
    {
        return Err(EvalError::BadName(name.to_owned()));
    }
    let path = Path::new(EVAL_DIR).join(format!("{name}.json"));
    run(
        runs,
        concurrency,
        filter,
        RunOutput { name, path: &path },
        config_path,
        resume,
        serve,
    )
    .await
}

/// Where a run's artifacts are filed: the run `name` (the trend record correlates a line back to its
/// package by it) and its resolved `eval/<name>.json` path. The two travel together — the name names
/// the run and the path is derived from it — so they ride as one parameter rather than two.
struct RunOutput<'a> {
    name: &'a str,
    path: &'a Path,
}

/// Run the suite; returns whether every gating oracle held (the exit-code signal).
async fn run(
    runs: u32,
    concurrency: usize,
    filter: Option<&str>,
    output: RunOutput<'_>,
    config_path: &Path,
    resume: bool,
    serve: ServeConfig,
) -> Result<bool, EvalError> {
    let RunOutput { name, path: out } = output;
    let config = EnvConfig::load(config_path).map_err(|source| EvalError::LoadConfig {
        path: config_path.to_path_buf(),
        source: Box::new(source),
    })?;
    if config.model.endpoint.is_empty() {
        // Skip with a clear signal rather than fail (spec §Validation → the model-gated lane).
        tracing::warn!("skipping the reply lane: no model endpoint configured");
        return Ok(true);
    }

    // Wrap both seams in the retrying adapters so a transient endpoint outage (a host rebuild,
    // a serving-layer restart) backs off and recovers rather than aborting whichever runs coincide
    // with it and counting them as quality failures (see `retry`).
    let model: Arc<dyn ModelClient> = Arc::new(RetryingModel::new(Arc::new(OpenAiClient::new(
        &config.model,
    ))));
    let embedder: Option<Arc<dyn Embedder>> = (!config.embedding.endpoint.is_empty()).then(|| {
        Arc::new(RetryingEmbedder::new(Arc::new(OpenAiEmbedder::new(
            &config.embedding,
        )))) as Arc<dyn Embedder>
    });
    // A test-only MCP host with a "fetch" server whose `markdown` tool returns a large canned
    // article — the real-world path the content limit guards against (an agent fetches a page and
    // tries to paste the whole thing into memory). Pure in-memory: no subprocess, no network.
    let fetch_host = fetch_fixture::fetch_host();
    let deps = RunDeps {
        model,
        embedder,
        dimensions: config.embedding.dimensions,
        mcp: Some(fetch_host),
    };

    let mut scenarios = crate::scenarios::all();
    if let Some(filter) = filter {
        // Comma-separated substrings, matched by OR — so a diverse subset can be selected in one run
        // (e.g. `--scenario tag_room,recall,flush`), not just a single name.
        let needles: Vec<&str> = filter
            .split(',')
            .map(str::trim)
            .filter(|needle| !needle.is_empty())
            .collect();
        scenarios.retain(|scenario| {
            let name = scenario.meta().name;
            needles.iter().any(|needle| name.contains(needle))
        });
    }
    tracing::info!(
        scenarios = scenarios.len(),
        runs,
        concurrency,
        "running the eval suite"
    );

    // The scenarios that will actually run; the manifest and the live log's scenario indices are over
    // this list.
    let active =
        crate::harness::active_scenarios(scenarios, deps.embedder.is_some(), deps.mcp.is_some());
    let scenario_metas: Vec<ScenarioMeta> = active.iter().map(|scenario| scenario.meta()).collect();

    let started_at_ms = live::now_ms();
    let meta = RunMeta {
        harness_version: env!("CARGO_PKG_VERSION").to_owned(),
        git_sha: git_sha(),
        git_dirty: git_dirty(),
        model_id: config.model.llm.clone(),
        embedding_model: (!config.embedding.endpoint.is_empty())
            .then(|| config.embedding.model.clone()),
        scenario_filter: filter.map(str::to_owned),
        started_at_ms,
        // Stamped for real on `finish`; the manifest carries the start so the viewer has a clock.
        finished_at_ms: started_at_ms,
        runs_per_scenario: runs,
        concurrency: concurrency as u32,
        rejudged_from: None,
        resumed_from: None,
    };

    // The resumable, tailable form while the run is in flight: one JSON line per live event, beside the
    // final package. Its presence (without the package) is itself the "this run is incomplete" signal.
    let sidecar = out.with_extension("jsonl");
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).map_err(|source| EvalError::WriteOutput {
            path: parent.to_path_buf(),
            source,
        })?;
    }

    // Resume continues the existing sidecar (its manifest, its completed runs) and skips them; a fresh
    // run seeds a new one from the manifest just built.
    let sink = if resume && sidecar.exists() {
        let state = live::read_sidecar(&sidecar)?;
        tracing::info!(completed = state.completed.len(), path = %sidecar.display(), "resuming from sidecar");
        Arc::new(live::EvalSink::resume(state, &sidecar)?)
    } else {
        Arc::new(live::EvalSink::new(meta, scenario_metas, &sidecar)?)
    };
    let done = sink.done_runs();

    // Serve the live stream before warming up the model endpoints, so a viewer connecting at launch
    // sees the scoreboard (the plan) immediately — not a dead page while the inference server warms.
    let serving = serve
        .addr
        .map(|addr| tokio::spawn(crate::serve::serve(addr, sink.clone())));

    // Warm the endpoints before the clock starts, so the first run isn't charged for cold-start.
    tracing::info!("warming up the model endpoints");
    crate::harness::warm_up(&deps).await;

    crate::harness::run_all(deps, active, runs, concurrency, sink.clone(), done).await?;
    sink.finish(live::now_ms())?;

    let package = sink.package();

    let all_gates_held = package.scenarios.iter().all(|report| {
        report
            .meta
            .bar
            .holds(report.aggregate.gating_rate, report.aggregate.gating_passed)
    });
    for report in &package.scenarios {
        tracing::info!(
            scenario = %report.meta.name,
            rate = report.aggregate.rate,
            gating = report.aggregate.gating_passed,
            wall_p50_ms = report.aggregate.wall_clock_ms.p50,
            latency_p50_ms = report.aggregate.latency_ms.p50,
            "scenario result"
        );
    }

    // Fold to the canonical package, then drop the sidecar — write fully before unlinking, so a crash
    // between leaves either a resumable sidecar or a complete package, never neither.
    write_package(&package, out)?;
    crate::history::append_history(name, &package)?;
    std::fs::remove_file(&sidecar).ok();

    // Only with `--serve-after-completion` does the process stay up after the run, so the operator can
    // review the final state live; the in-memory sink still answers new connections with the complete
    // package, and Ctrl-C exits. By default the run drops the serving task and exits with the gating
    // signal as soon as it finishes, so a background or scripted run never blocks.
    if serve.after_completion
        && let Some(serving) = serving
    {
        tracing::info!("run complete; serving the final state — Ctrl-C to exit");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            result = serving => {
                if let Ok(Err(error)) = result {
                    tracing::error!(%error, "the live serve ended with an error");
                }
            }
        }
    }
    Ok(all_gates_held)
}

pub(crate) fn write_package(package: &EvalPackage, out: &Path) -> Result<(), EvalError> {
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).map_err(|source| EvalError::WriteOutput {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    // Write the temp beside the target, then rename — an atomic swap, so a reader (and the resume
    // check) never sees a half-written package.
    let json = serde_json::to_vec_pretty(package)?;
    let tmp = out.with_extension("json.tmp");
    std::fs::write(&tmp, json).map_err(|source| EvalError::WriteOutput {
        path: tmp.clone(),
        source,
    })?;
    std::fs::rename(&tmp, out).map_err(|source| EvalError::WriteOutput {
        path: out.to_path_buf(),
        source,
    })?;
    tracing::info!(path = %out.display(), "wrote eval package");
    Ok(())
}

pub(crate) fn git_sha() -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// Whether the working tree had uncommitted changes to tracked files — `git status --porcelain`
/// with untracked files excluded, since a stray scratch file would otherwise flag every run dirty
/// forever and drain the flag of meaning; only tracked modifications can differ from the recorded
/// sha. Best-effort like [`git_sha`]: an unavailable or failing git reads as clean, so a run outside
/// a repository does not falsely flag itself dirty.
pub(crate) fn git_dirty() -> bool {
    let Ok(output) = std::process::Command::new("git")
        .args(["status", "--porcelain", "--untracked-files=no"])
        .output()
    else {
        return false;
    };
    output.status.success() && !String::from_utf8_lossy(&output.stdout).trim().is_empty()
}
