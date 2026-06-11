//! The zuihitsu eval harness: drives reply-lane scenarios against the real model, measures them, and
//! emits an eval package (spec §Validation → the reply lane is a standalone harness). `run` produces a
//! package and a tracked metrics line; `export-types` writes the TypeScript type contract the viewer
//! consumes.

mod analysis;
mod context;
mod error;
mod harness;
mod judge;
mod package;
mod scenario;
mod scenarios;

use std::{
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
    package::{EvalPackage, RunMeta},
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
        /// Where to write the full eval package.
        #[arg(long, default_value = "eval/latest.json")]
        out: PathBuf,
        /// The agent config to load the model/embedding endpoints from.
        #[arg(long, default_value = "config.toml")]
        config: PathBuf,
    },
    /// Export the eval-package and event-log types to TypeScript (the viewer's type contract).
    ExportTypes {
        /// The directory to write the `.ts` bindings into.
        dir: PathBuf,
    },
}

#[tokio::main]
async fn main() -> ExitCode {
    init_tracing();
    match Cli::parse().command {
        Command::ExportTypes { dir } => export_types(&dir),
        Command::Run {
            runs,
            concurrency,
            scenario,
            out,
            config,
        } => match run(runs, concurrency, scenario.as_deref(), &out, &config).await {
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
    match EvalPackage::export_all_to(dir) {
        Ok(()) => {
            println!("exported the eval-package types to {}", dir.display());
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("eval: exporting types failed: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Run the suite; returns whether every gating oracle held (the exit-code signal).
async fn run(
    runs: u32,
    concurrency: usize,
    filter: Option<&str>,
    out: &Path,
    config_path: &Path,
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

    let model: Arc<dyn ModelClient> = Arc::new(OpenAiClient::new(&config.model));
    let embedder: Option<Arc<dyn Embedder>> = (!config.embedding.endpoint.is_empty())
        .then(|| Arc::new(OpenAiEmbedder::new(&config.embedding)) as Arc<dyn Embedder>);
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

    // Warm the endpoints before the clock starts, so the first run isn't charged for cold-start.
    tracing::info!("warming up the model endpoints");
    harness::warm_up(&deps).await;

    let started_at_ms = now_ms();
    let reports = harness::run_all(deps, scenarios, runs, concurrency).await;
    let finished_at_ms = now_ms();

    let all_gates_held = reports.iter().all(|report| report.aggregate.gating_passed);
    for report in &reports {
        tracing::info!(
            scenario = %report.meta.name,
            rate = report.aggregate.rate,
            gating = report.aggregate.gating_passed,
            wall_p50_ms = report.aggregate.wall_clock_ms.p50,
            latency_p50_ms = report.aggregate.latency_ms.p50,
            "scenario result"
        );
    }

    let package = EvalPackage {
        meta: RunMeta {
            harness_version: env!("CARGO_PKG_VERSION").to_owned(),
            git_sha: git_sha(),
            model_id: config.model.llm.clone(),
            embedding_model: (!config.embedding.endpoint.is_empty())
                .then(|| config.embedding.model.clone()),
            started_at_ms,
            finished_at_ms,
            runs_per_scenario: runs,
            concurrency: concurrency as u32,
        },
        scenarios: reports,
    };

    write_package(&package, out)?;
    append_history(&package)?;
    Ok(all_gates_held)
}

fn write_package(package: &EvalPackage, out: &Path) -> Result<(), EvalError> {
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).map_err(|source| EvalError::WriteOutput {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    let json = serde_json::to_vec_pretty(package)?;
    std::fs::write(out, json).map_err(|source| EvalError::WriteOutput {
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
