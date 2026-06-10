//! The zuihitsu binary. Run with no subcommand it boots the long-running HTTP server that hosts the
//! agent (see [`serve`]); with a subcommand it is the operator CLI — a client of that running server
//! (see [`client`]), reaching the agent through its `/control` API rather than opening the store
//! directly (only the server holds the single-writer log lock). A "CLI debugger": inspection
//! subcommands print their JSON response to stdout; diagnostics go through `tracing` to stderr.

use std::{
    path::{Path, PathBuf},
    process::ExitCode,
};

use clap::{Parser, Subcommand};
use serde::Serialize;
use tracing_subscriber::{EnvFilter, fmt};
use zuihitsu::{ConfigError, GenesisStatus, Rollout, SeedSelf, config::EnvConfig};

use crate::client::{Client, ClientError};

mod client;
mod serve;

fn main() -> ExitCode {
    run()
}

/// Operator client for a Zuihitsu agent (and, with no subcommand, the server itself).
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
    /// Inspect a memory by name (e.g. `self`, `person/dave@discord`).
    Memory {
        #[arg(long)]
        name: String,
    },
    /// List the memories in a namespace (e.g. `person/`).
    Memories {
        #[arg(long)]
        prefix: String,
    },
    /// Show a memory's content entries by name.
    Entries {
        #[arg(long)]
        name: String,
    },
    /// List a conversation's sessions, oldest first.
    Sessions {
        #[arg(long)]
        platform: String,
        #[arg(long)]
        scope: String,
    },
    /// List the memories with a recurring occurrence.
    Recurring,
    /// List the recorded belief arbitrations.
    Arbitrations,
    /// List the recorded model interactions (per-call request, deliberation, tokens, and latency).
    Interactions,
    /// Write a graph snapshot now (a checkpoint to speed the next cold boot, or before an experiment).
    Snapshot,
    /// Print the agent's current behavioral settings.
    Settings,
    /// Replace the behavioral settings from a JSON file.
    SetSettings {
        #[arg(long)]
        file: PathBuf,
    },
    /// Send one operator message of the imprint interview (prints the agent's reply).
    Imprint {
        #[arg(long)]
        text: String,
    },
    /// Deliver a participant message and print the agent's reply.
    Send {
        #[arg(long)]
        platform: String,
        #[arg(long)]
        scope: String,
        #[arg(long)]
        sender: String,
        #[arg(long)]
        text: String,
        /// A present participant; repeatable. The sender is always treated as present.
        #[arg(long = "present")]
        present: Vec<String>,
    },
    /// Note a participant arriving mid-session.
    Join {
        #[arg(long)]
        platform: String,
        #[arg(long)]
        scope: String,
        #[arg(long)]
        participant: String,
    },
}

pub fn run() -> ExitCode {
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
/// `RUST_LOG` (default `info`). Inspection subcommands print machine-readable JSON to stdout.
fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();
}

fn dispatch(cli: &Cli) -> Result<(), CliError> {
    let Some(command) = &cli.command else {
        return serve(&cli.config);
    };
    let config = EnvConfig::load(&cli.config).map_err(|source| CliError::LoadConfig {
        path: cli.config.clone(),
        source,
    })?;
    let client = Client::new(config.serving.bind);
    match command {
        Command::Create {
            name,
            persona,
            seed,
        } => create(&client, name, persona, seed),
        Command::Status => status(&client),
        Command::Memory { name } => memory(&client, name),
        Command::Memories { prefix } => print_json(&client.memories(prefix)?),
        Command::Entries { name } => print_json(&client.entries(name)?),
        Command::Sessions { platform, scope } => print_json(&client.sessions(platform, scope)?),
        Command::Recurring => print_json(&client.recurring()?),
        Command::Arbitrations => print_json(&client.arbitrations()?),
        Command::Interactions => print_json(&client.interactions()?),
        Command::Snapshot => print_json(&client.snapshot()?),
        Command::Settings => print_json(&client.settings()?),
        Command::SetSettings { file } => set_settings(&client, file),
        Command::Imprint { text } => print_json(&client.imprint(text)?),
        Command::Send {
            platform,
            scope,
            sender,
            text,
            present,
        } => print_json(&client.send(platform, scope, sender, text, present)?),
        Command::Join {
            platform,
            scope,
            participant,
        } => {
            client.join(platform, scope, participant)?;
            tracing::info!(%platform, %scope, %participant, "noted join");
            Ok(())
        }
    }
}

/// Boot the long-running HTTP server (the primary operation).
fn serve(config_path: &Path) -> Result<(), CliError> {
    crate::serve::run_blocking(config_path).map_err(CliError::Serve)
}

fn create(client: &Client, name: &str, persona: &str, seed: &[String]) -> Result<(), CliError> {
    let seed = SeedSelf {
        agent_name: name.to_owned(),
        persona: persona.to_owned(),
        seed_entries: seed.to_vec(),
    };
    match client.create_agent(&seed)? {
        Rollout::Created { events_emitted } => {
            tracing::info!(agent = %seed.agent_name, events = events_emitted, "created agent");
        }
        Rollout::AlreadyComplete => {
            tracing::info!("an agent already exists here; nothing to do");
        }
    }
    Ok(())
}

fn status(client: &Client) -> Result<(), CliError> {
    match client.genesis()? {
        GenesisStatus::Empty => {
            tracing::info!(
                "no agent here yet; run `zuihitsu create --name <name> --persona <persona>`"
            );
        }
        GenesisStatus::Incomplete => {
            tracing::warn!("genesis is incomplete; re-run `zuihitsu create` to resume it");
        }
        GenesisStatus::Complete => {
            tracing::info!("the agent is ready");
            if let Some(memory) = client.memory("self")?
                && !memory.description.is_empty()
            {
                tracing::info!(description = %memory.description, "self");
            }
        }
    }
    Ok(())
}

fn memory(client: &Client, name: &str) -> Result<(), CliError> {
    match client.memory(name)? {
        Some(view) => print_json(&view),
        None => {
            tracing::info!(%name, "no such memory");
            Ok(())
        }
    }
}

fn set_settings(client: &Client, file: &Path) -> Result<(), CliError> {
    let text = std::fs::read_to_string(file).map_err(|source| CliError::ReadFile {
        path: file.to_owned(),
        source,
    })?;
    let settings = serde_json::from_str(&text).map_err(|source| CliError::ParseSettings {
        path: file.to_owned(),
        source,
    })?;
    client.set_settings(&settings)?;
    tracing::info!(file = %file.display(), "settings updated");
    Ok(())
}

/// Print a response as pretty JSON to stdout — the machine-readable command output a debugger consumes.
fn print_json<T: Serialize>(value: &T) -> Result<(), CliError> {
    let json = serde_json::to_string_pretty(value).map_err(CliError::Render)?;
    println!("{json}");
    Ok(())
}

/// A CLI-level failure, naming the operation and the resource it was acting on.
#[derive(Debug)]
enum CliError {
    LoadConfig {
        path: PathBuf,
        source: ConfigError,
    },
    Serve(serve::ServeError),
    Client(ClientError),
    ReadFile {
        path: PathBuf,
        source: std::io::Error,
    },
    ParseSettings {
        path: PathBuf,
        source: serde_json::Error,
    },
    Render(serde_json::Error),
}

impl From<ClientError> for CliError {
    fn from(error: ClientError) -> Self {
        CliError::Client(error)
    }
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CliError::LoadConfig { path, source } => {
                write!(f, "could not load config from {}: {source}", path.display())
            }
            CliError::Serve(source) => write!(f, "the server exited with an error: {source}"),
            CliError::Client(source) => write!(f, "{source}"),
            CliError::ReadFile { path, source } => {
                write!(f, "could not read {}: {source}", path.display())
            }
            CliError::ParseSettings { path, source } => {
                write!(
                    f,
                    "could not parse settings from {}: {source}",
                    path.display()
                )
            }
            CliError::Render(source) => write!(f, "could not render the response: {source}"),
        }
    }
}

impl std::error::Error for CliError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            CliError::LoadConfig { source, .. } => Some(source),
            CliError::Serve(source) => Some(source),
            CliError::Client(source) => Some(source),
            CliError::ReadFile { source, .. } => Some(source),
            CliError::ParseSettings { source, .. } => Some(source),
            CliError::Render(source) => Some(source),
        }
    }
}
