//! The zuihitsu binary: run with no subcommand it boots the long-running HTTP server that hosts the
//! agent (see [`http_server`]); with a subcommand it is the operator CLI — a client of that running
//! server, reaching the agent through its `/control` API rather than opening the store directly (only
//! the server holds the single-writer log lock). A "CLI console": inspection subcommands print their
//! JSON response to stdout; diagnostics go through `tracing` to stderr. The commands are grouped under
//! three namespaces — [`cli::interact`] drives the agent, [`cli::state`] inspects its materialized
//! state, and [`cli::debug`] reads the event log directly — while `create`, `status`, `settings`, and
//! `set-settings` stay at the root. The parser and dispatch live here; the handlers live under [`cli`].

use std::{
    path::{Path, PathBuf},
    process::ExitCode,
};

use clap::{Parser, Subcommand};
use tracing_subscriber::{EnvFilter, fmt};
use zuihitsu::config::EnvConfig;

use crate::cli::{
    client::Client,
    debug::{self, DebugCommand},
    error::CliError,
    interact::{self, InteractCommand},
    root,
    state::{self, StateCommand},
};

mod cli;
mod http_server;

fn main() -> ExitCode {
    run()
}

/// Operator client for a zuihitsu agent (and, with no subcommand, the server itself).
#[derive(Parser)]
#[command(name = "zuihitsu", version, about)]
struct Cli {
    /// Path to the environmental config file (selects the instance).
    #[arg(long, default_value = "config.toml", global = true)]
    config: PathBuf,
    /// The operation to perform. With none, `zuihitsu` boots the long-running server.
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Create the agent, or resume an interrupted genesis.
    Create {
        /// The agent's name.
        #[arg(long)]
        name: String,
        /// A one-line persona.
        #[arg(long)]
        persona: String,
        /// A seed disposition entry; repeatable.
        #[arg(long = "seed")]
        seed: Vec<String>,
    },
    /// Report whether an agent exists and is ready.
    Status,
    /// Print the agent's current behavioral settings.
    Settings,
    /// Replace the behavioral settings from a JSON file.
    SetSettings {
        #[arg(long)]
        file: PathBuf,
    },
    /// Drive the agent: imprint, deliver a message, or note a join.
    #[command(subcommand)]
    Interact(InteractCommand),
    /// Inspect the agent's materialized state: memories, entries, sessions, and schedules.
    #[command(subcommand)]
    State(StateCommand),
    /// Diagnostics against the event log directly: events, briefs, reverts, and catalogues.
    #[command(subcommand)]
    Debug(DebugCommand),
}

fn run() -> ExitCode {
    init_tracing();
    let cli = Cli::parse();
    match dispatch(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}

/// Diagnostic and operational output goes through `tracing` to stderr, level controlled by
/// `RUST_LOG` (default `info`). Inspection subcommands print machine-readable JSON to stdout. Span
/// close events are emitted so the per-turn span (which records its outcome, duration, and counts
/// after the turn resolves) prints a summary line an operator can read off without re-reading the raw
/// event log (spec §Observability → per-turn spans).
fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    fmt()
        .with_env_filter(filter)
        .with_span_events(fmt::format::FmtSpan::CLOSE)
        .with_writer(std::io::stderr)
        .init();
}

fn dispatch(cli: &Cli) -> Result<(), CliError> {
    let Some(command) = &cli.command else {
        return http_server(&cli.config);
    };
    let config = EnvConfig::load(&cli.config).map_err(|source| CliError::LoadConfig { source })?;
    let client = Client::new(config.serving.bind);
    match command {
        Command::Create {
            name,
            persona,
            seed,
        } => root::create(&client, name, persona, seed),
        Command::Status => root::status(&client),
        Command::Settings => root::settings(&client),
        Command::SetSettings { file } => root::set_settings(&client, file),
        Command::Interact(command) => interact::dispatch(&client, command),
        Command::State(command) => state::dispatch(&client, command),
        Command::Debug(command) => debug::dispatch(&client, &config, command),
    }
}

/// Boot the long-running HTTP server (the primary operation, the default with no subcommand).
fn http_server(config_path: &Path) -> Result<(), CliError> {
    crate::http_server::run_blocking(config_path).map_err(CliError::HttpServer)
}
