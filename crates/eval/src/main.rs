//! The zuihitsu eval harness: drives reply-lane scenarios against the real model, measures them, and
//! emits an eval package (spec §Validation → the reply lane is a standalone harness). `run` produces a
//! package and a tracked metrics line; `export-types` writes the TypeScript type contract the viewer
//! consumes.

mod analysis;
mod analyze;
mod context;
mod error;
mod harness;
mod judge;
mod live;
mod package;
mod retry;
mod scenario;
mod scenarios;
mod serve;

use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
    process::ExitCode,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use clap::{Parser, Subcommand};
use serde::Serialize;
use ts_rs::TS;
use zuihitsu::{Embedder, EnvConfig, ModelClient, OpenAiClient, OpenAiEmbedder};

use crate::{
    context::RunDeps,
    error::EvalError,
    live::{EvalSink, LiveEvent},
    package::{EvalPackage, RunMeta, ScenarioMeta},
    retry::{RetryingEmbedder, RetryingModel},
};

#[derive(Parser)]
#[command(about = "The zuihitsu eval harness and its TypeScript type contract.")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the scenario suite against the configured model and write an eval package.
    Run {
        /// How many times to run each scenario.
        #[arg(long, default_value_t = 8)]
        runs: u32,
        /// At most this many runs in flight. Defaults to 1: the local endpoint serializes inference,
        /// so a second in-flight run only contends — measured at ~5x the per-request latency and ~2x
        /// the wall-clock of a serial pass.
        #[arg(long, default_value_t = 1)]
        concurrency: usize,
        /// Only run scenarios whose name contains one of these comma-separated substrings.
        #[arg(long)]
        scenario: Option<String>,
        /// The name this run is filed under: written to `eval/<name>.json` (with its `.jsonl`
        /// sidecar), so every run is kept in one place rather than scattered to arbitrary paths.
        /// Required — a bare filename, no path or extension.
        #[arg(long)]
        name: String,
        /// The agent config to load the model/embedding endpoints from.
        #[arg(long, default_value = "config.toml")]
        config: PathBuf,
        /// Resume an interrupted run from its `.jsonl` sidecar beside the output, driving only the
        /// runs it does not already hold. Ignored if no sidecar is present.
        #[arg(long)]
        resume: bool,
        /// Serve the run live over SSE for the console to watch — the scoreboard fills in as runs
        /// complete. On by default at `127.0.0.1:7878`; pass an address to bind somewhere else.
        /// Serving stops when the run finishes unless `--serve-after-completion` is set.
        #[arg(long, value_name = "ADDR", num_args = 0..=1, default_missing_value = DEFAULT_SERVE_ADDR)]
        serve: Option<SocketAddr>,
        /// Do not serve the run live; run to completion and exit.
        #[arg(long, conflicts_with = "serve")]
        no_serve: bool,
        /// Keep serving the final state after the run completes, until Ctrl-C, for reviewing the
        /// result live. By default serving stops when the run finishes.
        #[arg(long, conflicts_with = "no_serve")]
        serve_after_completion: bool,
    },
    /// Export the eval-package and event-log types to TypeScript (the viewer's type contract).
    ExportTypes {
        /// The directory to write the `.ts` bindings into.
        dir: PathBuf,
    },
    /// Read a written eval package: a per-scenario summary (with deltas against a baseline), or the
    /// complete deliberation traces of the runs that failed.
    Analyze {
        /// The package to read, e.g. `eval/scaffold-aggr-v4.json`.
        package: PathBuf,
        /// A baseline package to diff the summary against.
        #[arg(long, short)]
        baseline: Option<PathBuf>,
        /// Dump the failed runs' deliberation traces instead of the summary.
        #[arg(long, short)]
        failures: bool,
        /// Restrict to scenarios whose name contains this substring.
        #[arg(long, short)]
        scenario: Option<String>,
        /// With `--failures`, also print the events whose payload type contains this substring for
        /// each dumped run (e.g. `Scheduled`, `ContentAppended`, `TemporalResolved`), to pinpoint why
        /// a run failed at the event level.
        #[arg(long, short)]
        events: Option<String>,
        /// Cap the failed runs dumped per scenario (0 = all).
        #[arg(long, default_value_t = 0)]
        limit: usize,
        /// Clip long reasoning and scripts to this many characters (0 = full).
        #[arg(long, default_value_t = 600)]
        truncate: usize,
    },
}

#[tokio::main]
async fn main() -> ExitCode {
    init_tracing();
    match Cli::parse().command {
        Command::ExportTypes { dir } => export_types(&dir),
        Command::Analyze {
            package,
            baseline,
            failures,
            scenario,
            events,
            limit,
            truncate,
        } => match analyze::analyze(
            &package,
            baseline.as_deref(),
            failures,
            scenario.as_deref(),
            events.as_deref(),
            limit,
            truncate,
        ) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                tracing::error!("{error}");
                ExitCode::FAILURE
            }
        },
        Command::Run {
            runs,
            concurrency,
            scenario,
            name,
            config,
            resume,
            serve,
            no_serve,
            serve_after_completion,
        } => match run_named(
            runs,
            concurrency,
            scenario.as_deref(),
            &name,
            &config,
            resume,
            resolve_serve(serve, no_serve, serve_after_completion),
        )
        .await
        {
            Ok(all_gates_held) => {
                if all_gates_held {
                    ExitCode::SUCCESS
                } else {
                    tracing::error!("a gating safety oracle regressed");
                    ExitCode::FAILURE
                }
            }
            Err(error) => {
                tracing::error!("{error}");
                ExitCode::FAILURE
            }
        },
    }
}

fn export_types(dir: &Path) -> ExitCode {
    // The static package contract and the live stream's `LiveEvent` (its dependency trees overlap, so
    // the shared types regenerate identically); the console consumes both.
    match EvalPackage::export_all_to(dir).and_then(|()| LiveEvent::export_all_to(dir)) {
        Ok(()) => {
            println!(
                "exported the eval-package and live-event types to {}",
                dir.display()
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("eval: exporting types failed: {error}");
            ExitCode::FAILURE
        }
    }
}

/// The directory every eval run is filed under — kept together so runs are findable rather than
/// scattered to arbitrary paths (or `/tmp`, where they are lost to GC). Gitignored; only the small
/// `history.jsonl` trend record is tracked.
const EVAL_DIR: &str = "eval";

/// The address live serving binds when no override is given.
const DEFAULT_SERVE_ADDR: &str = "127.0.0.1:7878";

/// How a run serves its live view.
struct ServeConfig {
    /// Where to bind the SSE endpoint, or `None` to not serve at all (`--no-serve`).
    addr: Option<SocketAddr>,
    /// Keep serving the final state after the run completes (until Ctrl-C) rather than exiting.
    after_completion: bool,
}

/// Resolve the live-serving config from the flags. Serving is on by default — `--no-serve` opts out,
/// and an explicit `--serve` overrides the bind address — and stops when the run finishes unless
/// `--serve-after-completion` keeps it up for review.
fn resolve_serve(serve: Option<SocketAddr>, no_serve: bool, after_completion: bool) -> ServeConfig {
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
async fn run_named(
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
    let out = Path::new(EVAL_DIR).join(format!("{name}.json"));
    run(runs, concurrency, filter, &out, config_path, resume, serve).await
}

/// Run the suite; returns whether every gating oracle held (the exit-code signal).
async fn run(
    runs: u32,
    concurrency: usize,
    filter: Option<&str>,
    out: &Path,
    config_path: &Path,
    resume: bool,
    serve: ServeConfig,
) -> Result<bool, EvalError> {
    let config = EnvConfig::load(config_path).map_err(|source| EvalError::LoadConfig {
        path: config_path.to_path_buf(),
        source,
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
    let deps = RunDeps {
        model,
        embedder,
        dimensions: config.embedding.dimensions,
    };

    let mut scenarios = scenarios::all();
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
    let active = harness::active_scenarios(scenarios, deps.embedder.is_some());
    let scenario_metas: Vec<ScenarioMeta> = active.iter().map(|scenario| scenario.meta()).collect();

    let started_at_ms = now_ms();
    let meta = RunMeta {
        harness_version: env!("CARGO_PKG_VERSION").to_owned(),
        git_sha: git_sha(),
        model_id: config.model.llm.clone(),
        embedding_model: (!config.embedding.endpoint.is_empty())
            .then(|| config.embedding.model.clone()),
        started_at_ms,
        // Stamped for real on `finish`; the manifest carries the start so the viewer has a clock.
        finished_at_ms: started_at_ms,
        runs_per_scenario: runs,
        concurrency: concurrency as u32,
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
    let (sink, done) = if resume && sidecar.exists() {
        let state = live::read_sidecar(&sidecar)?;
        tracing::info!(completed = state.completed.len(), path = %sidecar.display(), "resuming from sidecar");
        let sink = Arc::new(EvalSink::resume(state, &sidecar)?);
        let done = sink.done_runs();
        (sink, done)
    } else {
        let sink = Arc::new(EvalSink::new(meta, scenario_metas, &sidecar)?);
        (sink, std::collections::HashSet::new())
    };

    // Serve the live stream before warming up the model endpoints, so a viewer connecting at launch
    // sees the scoreboard (the plan) immediately — not a dead page while the inference server warms.
    let serving = serve
        .addr
        .map(|addr| tokio::spawn(serve::serve(addr, sink.clone())));

    // Warm the endpoints before the clock starts, so the first run isn't charged for cold-start.
    tracing::info!("warming up the model endpoints");
    harness::warm_up(&deps).await;

    harness::run_all(deps, active, runs, concurrency, sink.clone(), done).await?;
    sink.finish(now_ms())?;

    let package = sink.package();

    let all_gates_held = package
        .scenarios
        .iter()
        .all(|report| report.aggregate.gating_passed);
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
    append_history(&package)?;
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

fn write_package(package: &EvalPackage, out: &Path) -> Result<(), EvalError> {
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

fn append_history(package: &EvalPackage) -> Result<(), EvalError> {
    use std::io::Write as _;

    /// One compact, deterministically-ordered line per run, appended to the tracked history (spec
    /// §Validation → the tracked metrics trend).
    #[derive(Serialize)]
    struct HistoryLine {
        ts_ms: i64,
        git_sha: Option<String>,
        model_id: String,
        runs_per_scenario: u32,
        scenarios: Vec<HistoryScenario>,
    }

    #[derive(Serialize)]
    struct HistoryScenario {
        name: String,
        rate: f64,
        gating_passed: bool,
        wall_clock_p50_ms: u64,
        latency_p50_ms: u64,
        total_tokens_mean: u64,
        prompt_tokens_mean: u64,
        completion_tokens_mean: u64,
    }

    let line = HistoryLine {
        ts_ms: package.meta.finished_at_ms,
        git_sha: package.meta.git_sha.clone(),
        model_id: package.meta.model_id.clone(),
        runs_per_scenario: package.meta.runs_per_scenario,
        scenarios: package
            .scenarios
            .iter()
            .map(|report| HistoryScenario {
                name: report.meta.name.clone(),
                // Round so an unchanged result produces an identical line (clean diffs/appends).
                rate: (report.aggregate.rate * 1000.0).round() / 1000.0,
                gating_passed: report.aggregate.gating_passed,
                wall_clock_p50_ms: report.aggregate.wall_clock_ms.p50.round() as u64,
                latency_p50_ms: report.aggregate.latency_ms.p50.round() as u64,
                total_tokens_mean: report.aggregate.tokens.total_mean.round() as u64,
                prompt_tokens_mean: report.aggregate.tokens.prompt_mean.round() as u64,
                completion_tokens_mean: report.aggregate.tokens.completion_mean.round() as u64,
            })
            .collect(),
    };
    let path = Path::new("eval/history.jsonl");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| EvalError::WriteOutput {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|source| EvalError::WriteOutput {
            path: path.to_path_buf(),
            source,
        })?;
    let mut json = serde_json::to_string(&line)?;
    json.push('\n');
    file.write_all(json.as_bytes())
        .map_err(|source| EvalError::WriteOutput {
            path: path.to_path_buf(),
            source,
        })?;
    Ok(())
}

fn git_sha() -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_millis() as i64)
        .unwrap_or(0)
}

fn init_tracing() {
    use tracing_subscriber::{EnvFilter, fmt};
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init();
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_SERVE_ADDR, resolve_serve};

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
}
