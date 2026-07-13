//! The zuihitsu eval harness: drives reply-lane scenarios against the real model, measures them, and
//! emits an eval package (spec §Validation → the reply lane is a standalone harness). `run` produces a
//! package and a tracked metrics line; `export-types` writes the TypeScript type contract the viewer
//! consumes.

mod analysis;
mod analyze;
mod context;
mod error;
mod event_render;
mod executor;
mod fetch_fixture;
mod harness;
mod history;
mod judge;
mod live;
mod package;
mod replay;
mod retry;
mod run;
mod scenario;
mod scenarios;
mod serve;
mod step;

use std::{
    io::Write,
    net::SocketAddr,
    path::{Path, PathBuf},
    process::ExitCode,
};

use clap::{Parser, Subcommand};
use ts_rs::TS;

use crate::{
    live::LiveEvent,
    package::EvalPackage,
    replay::{ReplayMode, ReplayRequest},
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
        /// With `--resume`, also re-drive the completed runs bearing the infrastructure-failure
        /// signature — the model backend was unreachable for the run's whole life, so every turn
        /// deferred and no reply was produced — replacing each poisoned record in the package rather
        /// than keeping it. The signature is detected structurally from the recorded journal and
        /// metrics, never from an error string. Oracle-failed runs — whose turns all ran but whose
        /// verdicts missed — are legitimate data and are never retried, since redoing them would bias
        /// the suite's rates toward passing. Works on both an interrupted sidecar and a
        /// finished-but-poisoned one; a heal that finds nothing to re-drive is a clean no-op.
        #[arg(long, requires = "resume")]
        retry_infra_failed: bool,
        /// Serve the run live over SSE for the console to watch — the scoreboard fills in as runs
        /// complete. On by default at `127.0.0.1:7878`; pass an address to bind somewhere else.
        /// Serving stops when the run finishes unless `--serve-after-completion` is set.
        #[arg(long, value_name = "ADDR", num_args = 0..=1, default_missing_value = run::DEFAULT_SERVE_ADDR)]
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
        /// Print the relation-vocabulary projection instead of the summary: every relation used, whether
        /// it was seeded at genesis, its use count and namespace shapes, and the relations coined outside
        /// genesis (the drift signal). Respects `--scenario`.
        #[arg(long, short = 'r')]
        relations: bool,
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
    /// Read a recorded run's events, grouped under the journal steps that produced them — the step
    /// index shown is the coordinate `replay --mode resume --step` takes.
    Events {
        /// The package to read, e.g. `eval/content-limit-n10-v2.json`.
        package: PathBuf,
        /// Restrict to the scenario whose name contains this substring. Required when the package holds
        /// more than one scenario.
        #[arg(long, short)]
        scenario: Option<String>,
        /// The run index within the scenario (the `runs[N]` position). Defaults to 0.
        #[arg(long, short, default_value_t = 0)]
        run: usize,
        /// Clip each event's summary to this many characters (0 = full).
        #[arg(long, default_value_t = 120)]
        truncate: usize,
    },
    /// Replay a recorded package: re-assess it against the current criteria (`rejudge`), or rewind one
    /// run to a step and redo the rest of the scenario live (`resume`). Never writes trend history. Not
    /// to be confused with `run --resume`, which continues an interrupted suite run.
    Replay {
        /// The package to replay.
        package: PathBuf,
        /// rejudge: re-assess the recorded runs against the current criteria without re-running the
        /// model — for testing how oracle or judge changes reclassify an existing eval.
        /// resume: rewind one run to `--step` and redo the rest of the scenario live from that point
        /// with the current code and model.
        #[arg(long, value_enum)]
        mode: ReplayMode,
        /// Restrict to scenarios whose name contains this substring (required for resume).
        #[arg(long, short)]
        scenario: Option<String>,
        /// resume only: the run index to resume.
        #[arg(long, short)]
        run: Option<usize>,
        /// resume only: the last journal step to keep; everything after it is redone live
        /// (keep-semantics, matching the main CLI's `revert --seq`).
        #[arg(long)]
        step: Option<u32>,
        /// The agent config to load the model/embedding endpoints from.
        #[arg(long, default_value = "config.toml")]
        config: PathBuf,
        /// File the result under `eval/<name>.json` (like `run --name`). Optional for rejudge (report
        /// only when absent); resume defaults to a derived name recorded in the report.
        #[arg(long)]
        name: Option<String>,
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
            relations,
            scenario,
            events,
            limit,
            truncate,
        } => match analyze::analyze(analyze::AnalyzeRequest {
            package: &package,
            baseline: baseline.as_deref(),
            failures,
            relations,
            scenario: scenario.as_deref(),
            events: events.as_deref(),
            limit,
            truncate,
        }) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                tracing::error!("{error}");
                ExitCode::FAILURE
            }
        },
        Command::Events {
            package,
            scenario,
            run,
            truncate,
        } => match replay::events(&package, scenario.as_deref(), run, truncate) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                tracing::error!("{error}");
                ExitCode::FAILURE
            }
        },
        Command::Replay {
            package,
            mode,
            scenario,
            run,
            step,
            config,
            name,
        } => match replay::replay(ReplayRequest {
            package: &package,
            mode,
            scenario: scenario.as_deref(),
            run,
            step,
            config: &config,
            name: name.as_deref(),
        })
        .await
        {
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
            retry_infra_failed,
            serve,
            no_serve,
            serve_after_completion,
        } => match run::run_named(
            runs,
            concurrency,
            scenario.as_deref(),
            &name,
            &config,
            run::Resume {
                enabled: resume,
                retry_infra_failed,
            },
            run::resolve_serve(serve, no_serve, serve_after_completion),
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
            package::Bar::Gating { min_rate } if min_rate >= 1.0 => {
                format!("{bar_gating}gating{bar_gating:#}")
            }
            package::Bar::Gating {
                min_rate: threshold,
            } => {
                format!("{bar_gating}gate ≥{threshold:.2}{bar_gating:#}")
            }
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
        progress::TurnProgress,
    };
    let export = EvalPackage::export_all_to(dir)
        .and_then(|()| LiveEvent::export_all_to(dir))
        .and_then(|()| Namespace::export_all_to(dir))
        .and_then(|()| NamespacedMemoryName::export_all_to(dir))
        .and_then(|()| TurnOutcome::export_all_to(dir))
        .and_then(|()| BackendHealth::export_all_to(dir))
        .and_then(|()| TurnProgress::export_all_to(dir))
        .map_err(|error| error.to_string())
        .and_then(|()| write_console_constants(dir).map_err(|error| error.to_string()));
    match export {
        Ok(()) => {
            println!(
                "exported the eval-package and live-event types, and the console constants, to {}",
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

/// Emit the Rust constants the console needs as runtime *values* (ts-rs exports types, not consts),
/// so Rust stays the single source of truth for values that are load-bearing on both sides. Today
/// that is the [`DIRECT_PLATFORM`](zuihitsu::ids::DIRECT_PLATFORM) key: identity resolution merges an
/// arrival on it under operator authority (spec §Cross-platform identity), and the console builds its
/// own room locators with it — a drift between the two would silently break that reconciliation.
fn write_console_constants(dir: &Path) -> std::io::Result<()> {
    let contents = format!(
        "// Generated by `eval export-types` — do not edit. Rust constants the console consumes as \
         values.\n\n\
         /// The platform key for the operator's own direct interface (Rust `ids::DIRECT_PLATFORM`).\n\
         export const DIRECT_PLATFORM = {:?};\n",
        zuihitsu::ids::DIRECT_PLATFORM,
    );
    std::fs::write(dir.join("constants.ts"), contents)
}

fn init_tracing() {
    use tracing_subscriber::{EnvFilter, fmt};
    // html5ever (under the readability extraction) warns about foreign-namespace nodes on every
    // serialisation of a page that carries them — capped at error, as in the agent binary.
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,html5ever=error"));
    let _ = fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init();
}

#[cfg(test)]
mod tests;
