//! The CLI control client: an operator-authority client over the agent server. Run with no
//! subcommand, `zuihitsu` boots the long-running HTTP server (the `serve` feature); with a subcommand
//! it acts on the instance the environmental config selects. Non-interactive and scriptable; the
//! guided creation wizard will live in the web frontend. Diagnostics go through `tracing` to stderr —
//! the CLI is an operator/diagnostic tool.

use std::{
    io,
    path::{Path, PathBuf},
    process::ExitCode,
};

use clap::{Parser, Subcommand};
use tracing_subscriber::{EnvFilter, fmt};
use zuihitsu::{
    ConfigError, Graph, GraphError, MemoryName, SeedSelf, Server, ServerError, SqliteStore,
    StoreError, SystemClock,
    config::EnvConfig,
    genesis::{GenesisStatus, Rollout},
};

/// Operator control client for a Zuihitsu agent.
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
/// `RUST_LOG` (default `info`). The CLI is an operator tool; the web frontend is the user UI.
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
    let mut server = open_server(&cli.config)?;
    let status = server.boot().map_err(CliError::Boot)?;
    match command {
        Command::Create {
            name,
            persona,
            seed,
        } => create(&mut server, status, name, persona, seed),
        Command::Status => report_status(&mut server, status),
    }
}

/// Boot the long-running HTTP server (the primary operation). Available only with the `serve` feature;
/// otherwise the binary was built without serving support.
#[cfg(feature = "serve")]
fn serve(config_path: &Path) -> Result<(), CliError> {
    crate::serve::run_blocking(config_path).map_err(CliError::Serve)
}

#[cfg(not(feature = "serve"))]
fn serve(_config_path: &Path) -> Result<(), CliError> {
    Err(CliError::ServeUnavailable)
}

fn create(
    server: &mut Server,
    status: GenesisStatus,
    name: &str,
    persona: &str,
    seed: &[String],
) -> Result<(), CliError> {
    if status == GenesisStatus::Complete {
        tracing::info!("an agent already exists here; nothing to do");
        return Ok(());
    }
    let seed = SeedSelf {
        agent_name: name.to_owned(),
        persona: persona.to_owned(),
        seed_entries: seed.to_vec(),
    };
    match server
        .control()
        .create_agent(&seed)
        .map_err(CliError::CreateAgent)?
    {
        Rollout::Created { events_emitted } => {
            tracing::info!(agent = %seed.agent_name, events = events_emitted, "created agent");
        }
        Rollout::AlreadyComplete => {
            tracing::info!("an agent already exists here; nothing to do");
        }
    }
    Ok(())
}

fn report_status(server: &mut Server, status: GenesisStatus) -> Result<(), CliError> {
    match status {
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
            if let Some(memory) = server
                .control()
                .memory(MemoryName::SELF)
                .map_err(CliError::Inspect)?
                && !memory.description.is_empty()
            {
                tracing::info!(description = %memory.description, "self");
            }
        }
    }
    Ok(())
}

/// Open the instance the config selects, creating the database directories if absent.
fn open_server(config_path: &Path) -> Result<Server, CliError> {
    let config = EnvConfig::load(config_path).map_err(|source| CliError::LoadConfig {
        path: config_path.to_owned(),
        source,
    })?;
    ensure_parent_dir(&config.storage.event_log)?;
    ensure_parent_dir(&config.storage.graph)?;
    let store =
        SqliteStore::open(&config.storage.event_log).map_err(|source| CliError::OpenEventLog {
            path: config.storage.event_log.clone(),
            source,
        })?;
    let graph = Graph::open(&config.storage.graph).map_err(|source| CliError::OpenGraph {
        path: config.storage.graph.clone(),
        source,
    })?;
    Ok(Server::new(Box::new(store), graph, Box::new(SystemClock)))
}

fn ensure_parent_dir(path: &Path) -> Result<(), CliError> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|source| CliError::CreateDir {
            path: parent.to_owned(),
            source,
        })?;
    }
    Ok(())
}

/// A CLI-level failure, naming the operation and the resource it was acting on.
#[derive(Debug)]
enum CliError {
    LoadConfig {
        path: PathBuf,
        source: ConfigError,
    },
    CreateDir {
        path: PathBuf,
        source: io::Error,
    },
    OpenEventLog {
        path: PathBuf,
        source: StoreError,
    },
    OpenGraph {
        path: PathBuf,
        source: GraphError,
    },
    Boot(ServerError),
    CreateAgent(ServerError),
    Inspect(ServerError),
    /// The long-running server exited with an error.
    #[cfg(feature = "serve")]
    Serve(crate::serve::ServeError),
    /// `zuihitsu` was invoked with no subcommand, but built without the `serve` feature.
    #[cfg(not(feature = "serve"))]
    ServeUnavailable,
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CliError::LoadConfig { path, source } => {
                write!(f, "could not load config from {}: {source}", path.display())
            }
            CliError::CreateDir { path, source } => {
                write!(f, "could not create directory {}: {source}", path.display())
            }
            CliError::OpenEventLog { path, source } => {
                write!(
                    f,
                    "could not open the event log at {}: {source}",
                    path.display()
                )
            }
            CliError::OpenGraph { path, source } => {
                write!(
                    f,
                    "could not open the graph at {}: {source}",
                    path.display()
                )
            }
            CliError::Boot(source) => write!(f, "could not boot the agent: {source}"),
            CliError::CreateAgent(source) => write!(f, "could not create the agent: {source}"),
            CliError::Inspect(source) => write!(f, "could not inspect the agent: {source}"),
            #[cfg(feature = "serve")]
            CliError::Serve(source) => write!(f, "the server exited with an error: {source}"),
            #[cfg(not(feature = "serve"))]
            CliError::ServeUnavailable => write!(
                f,
                "this build has no `serve` feature; rebuild with it to run the server"
            ),
        }
    }
}

impl std::error::Error for CliError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            CliError::LoadConfig { source, .. } => Some(source),
            CliError::CreateDir { source, .. } => Some(source),
            CliError::OpenEventLog { source, .. } => Some(source),
            CliError::OpenGraph { source, .. } => Some(source),
            CliError::Boot(source) | CliError::CreateAgent(source) | CliError::Inspect(source) => {
                Some(source)
            }
            #[cfg(feature = "serve")]
            CliError::Serve(source) => Some(source),
            #[cfg(not(feature = "serve"))]
            CliError::ServeUnavailable => None,
        }
    }
}
