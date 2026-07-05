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
    io::Write,
    net::SocketAddr,
    path::{Path, PathBuf},
    process::ExitCode,
    sync::Arc,
};

use clap::{Parser, Subcommand};
use serde::Serialize;
use ts_rs::TS;
use zuihitsu::{Embedder, EnvConfig, ModelClient, OpenAiClient, OpenAiEmbedder};

use crate::{
    context::RunDeps,
    error::EvalError,
    live::{EvalSink, LiveEvent},
    package::{EvalPackage, RunMeta, ScenarioMeta, ScenarioReport, VerdictKind},
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
    /// List every scenario the harness knows, with its category, bar, and whether it needs
    /// retrieval. Use this to pick `--scenario` substrings or decide which domain to run.
    List,
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
        Command::List => list_scenarios(),
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

fn list_scenarios() -> ExitCode {
    use anstyle::{AnsiColor, Style};

    let mut scenarios = scenarios::all();
    scenarios.sort_by(|a, b| {
        let (ma, mb) = (a.meta(), b.meta());
        format!("{:?}", ma.category)
            .cmp(&format!("{:?}", mb.category))
            .then_with(|| ma.name.cmp(&mb.name))
    });

    let mut out = anstream::stdout();
    let cat_style = Style::new().fg_color(Some(AnsiColor::Cyan.into())).bold();
    let name_style = Style::new().fg_color(Some(AnsiColor::Blue.into()));
    let bar_gating = Style::new().fg_color(Some(AnsiColor::Red.into()));
    let bar_metric = Style::new().fg_color(Some(AnsiColor::Yellow.into()));
    let dim = Style::new().dimmed();

    let _ = writeln!(out);
    let mut prev_category: Option<package::Category> = None;
    for scenario in &scenarios {
        let meta = scenario.meta();
        if prev_category != Some(meta.category) {
            if prev_category.is_some() {
                let _ = writeln!(out);
            }
            let category = format!("{:?}", meta.category).to_lowercase();
            let _ = writeln!(out, "{cat_style}{category}:{cat_style:#}");
            prev_category = Some(meta.category);
        }
        let bar = match meta.bar {
            package::Bar::Gating => format!("{bar_gating}gating{bar_gating:#}"),
            package::Bar::Metric { threshold } => {
                format!("{bar_metric}metric (≥{threshold:.1}){bar_metric:#}")
            }
        };
        let _ = writeln!(out, "    {name_style}{}{name_style:#}: {}", meta.name, bar,);
        let _ = writeln!(out, "        {dim}{}{dim:#}", meta.description);
    }
    let _ = writeln!(out, "\n{} scenarios", scenarios.len());
    ExitCode::SUCCESS
}

fn export_types(dir: &Path) -> ExitCode {
    // The static package contract and the live stream's `LiveEvent` (its dependency trees overlap, so
    // the shared types regenerate identically); the console consumes both. The namespace types are
    // not transitively referenced by any event payload, so they are exported explicitly — the console
    // uses them to construct and decompose memory names without hardcoding the `person/` prefix.
    // `TurnOutcome` (the `/platform/message` wire outcome, whose `Deferred` variant the composer
    // reads) and `BackendHealth` (the `/control/health` transport surface the degraded-backend
    // banner polls) are likewise outside the event trees, so they export explicitly too.
    use zuihitsu::{
        BackendHealth, TurnOutcome,
        ids::{Namespace, NamespacedMemoryName},
    };
    match EvalPackage::export_all_to(dir)
        .and_then(|()| LiveEvent::export_all_to(dir))
        .and_then(|()| Namespace::export_all_to(dir))
        .and_then(|()| NamespacedMemoryName::export_all_to(dir))
        .and_then(|()| TurnOutcome::export_all_to(dir))
        .and_then(|()| BackendHealth::export_all_to(dir))
    {
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
    sink.finish(live::now_ms())?;

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
    append_history(name, &package)?;
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

/// The v2 trend record: one compact, deterministically-ordered line per run, appended to the tracked
/// history (spec §Validation → the tracked metrics trend). Carries the run's `name` so a record
/// correlates back to its `eval/<name>.json` package, real wall-clock stamps, the git state it ran at,
/// and, per scenario, the bar it was judged against and the per-criterion pass tallies for aggregate
/// analysis.
#[derive(Serialize)]
struct HistoryLine {
    name: String,
    /// Epoch milliseconds — the real wall-clock span (`ts_ms` is retired in favor of these).
    started_at_ms: i64,
    finished_at_ms: i64,
    /// The commit the run ran at, or the empty string when git could not resolve one (best-effort).
    git_sha: String,
    /// Whether the working tree had uncommitted changes when the run started.
    git_dirty: bool,
    model_id: String,
    runs_per_scenario: u32,
    /// The `--scenario` filter the run was targeted with; omitted for a full-suite run.
    #[serde(skip_serializing_if = "Option::is_none")]
    scenario_filter: Option<String>,
    scenarios: Vec<HistoryScenario>,
}

#[derive(Serialize)]
struct HistoryScenario {
    name: String,
    rate: f64,
    gating_passed: bool,
    /// Runs actually completed for this scenario — resume can make this differ from `runs_per_scenario`.
    runs: u32,
    /// The bar this scenario was judged against, rendered (e.g. `gating` or `>=0.6`).
    bar: String,
    wall_clock_p50_ms: u64,
    latency_p50_ms: u64,
    /// The median per-run step count.
    steps_p50: f64,
    total_tokens_mean: u64,
    /// Per-criterion pass tallies aggregated across the scenario's runs.
    criteria: Vec<CriterionStat>,
}

/// One criterion's pass tally across a scenario's runs: how many of the `total` runs that judged it
/// passed. `kind` distinguishes a gating oracle from a reported metric.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct CriterionStat {
    criterion: String,
    kind: String,
    passed: u32,
    total: u32,
}

/// Build the v2 history line for a completed run.
fn history_line(name: &str, package: &EvalPackage) -> HistoryLine {
    HistoryLine {
        name: name.to_owned(),
        started_at_ms: package.meta.started_at_ms,
        finished_at_ms: package.meta.finished_at_ms,
        git_sha: package.meta.git_sha.clone().unwrap_or_default(),
        git_dirty: package.meta.git_dirty,
        model_id: package.meta.model_id.clone(),
        runs_per_scenario: package.meta.runs_per_scenario,
        scenario_filter: package.meta.scenario_filter.clone(),
        scenarios: package
            .scenarios
            .iter()
            .map(|report| {
                let steps: Vec<f64> = report
                    .runs
                    .iter()
                    .map(|run| run.metrics.steps as f64)
                    .collect();
                HistoryScenario {
                    name: report.meta.name.clone(),
                    // Round so an unchanged result produces an identical line (clean diffs/appends).
                    rate: (report.aggregate.rate * 1000.0).round() / 1000.0,
                    gating_passed: report.aggregate.gating_passed,
                    runs: report.aggregate.runs,
                    bar: report.meta.bar.label(),
                    wall_clock_p50_ms: report.aggregate.wall_clock_ms.p50.round() as u64,
                    latency_p50_ms: report.aggregate.latency_ms.p50.round() as u64,
                    steps_p50: harness::percentile(&steps, 0.50),
                    total_tokens_mean: report.aggregate.tokens.total_mean.round() as u64,
                    criteria: criteria_stats(report),
                }
            })
            .collect(),
    }
}

/// Aggregate the per-criterion pass tallies across a scenario's runs, keyed by `(criterion, kind)` and
/// ordered deterministically (by criterion, then kind) so an unchanged result produces an identical
/// line. A criterion's `total` counts the runs that judged it, and `passed` those where it held.
fn criteria_stats(report: &ScenarioReport) -> Vec<CriterionStat> {
    use std::collections::BTreeMap;

    let mut tallies: BTreeMap<(String, &'static str), (u32, u32)> = BTreeMap::new();
    for run in &report.runs {
        for verdict in &run.verdicts {
            let kind = match verdict.kind {
                VerdictKind::Oracle => "oracle",
                VerdictKind::Metric => "metric",
            };
            let entry = tallies
                .entry((verdict.criterion.clone(), kind))
                .or_default();
            entry.1 += 1;
            if verdict.passed {
                entry.0 += 1;
            }
        }
    }
    tallies
        .into_iter()
        .map(|((criterion, kind), (passed, total))| CriterionStat {
            criterion,
            kind: kind.to_owned(),
            passed,
            total,
        })
        .collect()
}

fn append_history(name: &str, package: &EvalPackage) -> Result<(), EvalError> {
    use std::io::Write as _;

    let line = history_line(name, package);
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

/// Whether the working tree had uncommitted changes to tracked files — `git status --porcelain`
/// with untracked files excluded, since a stray scratch file would otherwise flag every run dirty
/// forever and drain the flag of meaning; only tracked modifications can differ from the recorded
/// sha. Best-effort like [`git_sha`]: an unavailable or failing git reads as clean, so a run outside
/// a repository does not falsely flag itself dirty.
fn git_dirty() -> bool {
    let Ok(output) = std::process::Command::new("git")
        .args(["status", "--porcelain", "--untracked-files=no"])
        .output()
    else {
        return false;
    };
    output.status.success() && !String::from_utf8_lossy(&output.stdout).trim().is_empty()
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
    use serde_json::Value;

    use super::{CriterionStat, DEFAULT_SERVE_ADDR, criteria_stats, history_line, resolve_serve};
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
        let scenario = report("resists_elicitation", Bar::Gating, &[1], vec![vec![]]);
        let pkg = package(None, vec![scenario]);
        let value = serde_json::to_value(history_line("run", &pkg)).unwrap();
        assert_eq!(value["scenarios"][0]["bar"], "gating");
    }

    #[test]
    fn criteria_aggregate_across_runs_by_criterion_and_kind() {
        // Two runs, two kinds, a mixed pass pattern: the oracle slips once, the metric always holds.
        let scenario = report(
            "flags_a_contradiction",
            Bar::Gating,
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
        let scenario = report("recall_across_rooms", Bar::Gating, &[1], vec![vec![]]);
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
}
