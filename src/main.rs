//! The zuihitsu binary. Run with no subcommand it boots the long-running HTTP server that hosts the
//! agent (see [`serve`]); with a subcommand it is the operator CLI — a client of that running server
//! (see [`client`]), reaching the agent through its `/control` API rather than opening the store
//! directly (only the server holds the single-writer log lock). A "CLI console": inspection
//! subcommands print their JSON response to stdout; diagnostics go through `tracing` to stderr.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::ExitCode,
};

use clap::{Parser, Subcommand};
use serde::Serialize;
use tracing_subscriber::{EnvFilter, fmt};
use zuihitsu::{
    ConfigError, Event, GenesisStatus, McpHost, McpTool, Rollout, SeedSelf, Seq, SqliteStore,
    StdioHost, Store, config::EnvConfig, event::EventPayload,
};

use crate::client::{Client, ClientError};

mod client;
mod serve;

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
    /// List the tools each configured MCP server exposes — spawns the servers directly (no running
    /// agent needed), so you can see a catalogue before narrowing it with `allow`/`deny`.
    Mcp,
    /// Inspect the event log directly, read-only — safe while the agent is running (it takes no lock).
    /// Lists events; with `--summary`, counts them by type and lays out the session timeline.
    Events {
        /// Show one event by seq, with its full payload pretty-printed (ignores the other filters).
        #[arg(long)]
        seq: Option<u64>,
        /// Only events at or after this seq.
        #[arg(long)]
        from: Option<u64>,
        /// Only events at or before this seq.
        #[arg(long)]
        to: Option<u64>,
        /// Only events of this type (case-insensitive, e.g. `SessionStarted`, `MemoryCreated`).
        #[arg(long = "type")]
        type_: Option<String>,
        /// Only events about this target — a conversation or memory id, or a prefix of one (so you can
        /// follow one room's turns, or one memory's history).
        #[arg(long)]
        target: Option<String>,
        /// Print each event's full JSON payload instead of a one-line summary.
        #[arg(long)]
        json: bool,
        /// Summarise: counts by type and the session timeline, instead of listing events.
        #[arg(long)]
        summary: bool,
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
        Command::Mcp => mcp(&config),
        Command::Events {
            seq,
            from,
            to,
            type_,
            target,
            json,
            summary,
        } => events(
            &config,
            EventQuery {
                seq: *seq,
                from: *from,
                to: *to,
                type_,
                target,
                json: *json,
                summary: *summary,
            },
        ),
    }
}

/// The filters and output mode for the `events` command, bundled so the inspection call stays one
/// argument rather than a fistful of options.
struct EventQuery<'a> {
    /// Show this one event's full payload, pretty-printed (ignores the rest).
    seq: Option<u64>,
    from: Option<u64>,
    to: Option<u64>,
    type_: &'a Option<String>,
    target: &'a Option<String>,
    json: bool,
    summary: bool,
}

/// Inspect the event log directly, opening it read-only so it is safe to read while the agent holds
/// the write lock. With `seq`, pretty-prints that one event's full payload; otherwise lists each event
/// (seq, type, and its target) or, with `summary`, counts events by type and lays out the session
/// timeline. `from`/`to` bound the seq range, `type_` filters by event type, `target` by the
/// conversation or memory the event is about, and `json` prints full payloads in the listing.
fn events(config: &EnvConfig, query: EventQuery) -> Result<(), CliError> {
    let EventQuery {
        seq,
        from,
        to,
        type_,
        target,
        json,
        summary,
    } = query;
    let path = config.storage.event_log();
    let store = SqliteStore::open_read_only(&path).map_err(|source| {
        CliError::Events(format!(
            "could not open the event log at {}: {source}",
            path.display()
        ))
    })?;
    let events = store
        .read_from(Seq(0))
        .map_err(|source| CliError::Events(format!("could not read the event log: {source}")))?;

    // A single event by seq: its whole payload, pretty-printed — the zoom-in the listing points at.
    if let Some(seq) = seq {
        let event = events
            .iter()
            .find(|event| event.seq.0 == seq)
            .ok_or_else(|| CliError::Events(format!("no event at seq {seq}")))?;
        let payload = serde_json::to_string_pretty(&event.payload).map_err(|source| {
            CliError::Events(format!("could not render the payload: {source}"))
        })?;
        println!("seq {} · {}\n{payload}", event.seq.0, event.payload.kind());
        return Ok(());
    }

    let mut events = events;
    if let Some(from) = from {
        events.retain(|event| event.seq.0 >= from);
    }
    if let Some(to) = to {
        events.retain(|event| event.seq.0 <= to);
    }
    if let Some(type_) = type_ {
        events.retain(|event| event.payload.kind().eq_ignore_ascii_case(type_));
    }
    if let Some(target) = target {
        events.retain(|event| {
            event
                .payload
                .target_id()
                .is_some_and(|id| id.starts_with(target.as_str()))
        });
    }

    if summary {
        print_event_summary(&events);
    } else {
        for event in &events {
            if json {
                let payload = serde_json::to_string(&event.payload).unwrap_or_default();
                println!(
                    "{:>6}  {:<26}  {payload}",
                    event.seq.0,
                    event.payload.kind()
                );
            } else {
                let target = event.payload.target_id().unwrap_or_default();
                println!("{:>6}  {:<26}  {target}", event.seq.0, event.payload.kind());
            }
        }
    }
    Ok(())
}

/// Counts by event type (commonest first), then every session boundary (`SessionStarted` /
/// `SessionEnded`) with its conversation — so dangling or duplicated sessions are visible at a glance.
fn print_event_summary(events: &[Event]) {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for event in events {
        *counts.entry(event.payload.kind()).or_default() += 1;
    }
    let mut by_count: Vec<(&str, usize)> = counts.into_iter().collect();
    by_count.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));

    println!("{} events", events.len());
    for (kind, count) in by_count {
        println!("  {count:>5}  {kind}");
    }

    println!("\nsessions");
    let mut any = false;
    for event in events {
        match &event.payload {
            EventPayload::SessionStarted {
                conversation,
                brief,
                ..
            } => {
                any = true;
                println!(
                    "  seq {:>5}  started  {}  brief {}ch",
                    event.seq.0,
                    conversation.0,
                    brief.len()
                );
            }
            EventPayload::SessionEnded { conversation, .. } => {
                any = true;
                println!("  seq {:>5}  ended    {}", event.seq.0, conversation.0);
            }
            _ => {}
        }
    }
    if !any {
        println!("  (none)");
    }
}

/// List the tools each configured MCP server exposes. Spawns the servers directly over stdio (no
/// running agent needed), snapshots each catalogue, and prints it as a readable listing — a server
/// that cannot be brought up reports its error and the rest still run, so one missing binary does not
/// hide the others. The operator reads this to choose an `allow`/`deny` projection.
fn mcp(config: &EnvConfig) -> Result<(), CliError> {
    if config.mcp.is_empty() {
        tracing::info!("no MCP servers configured; add an [mcp.<name>] block to the config");
        return Ok(());
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|source| CliError::Mcp(format!("could not start the async runtime: {source}")))?;
    runtime.block_on(async {
        let host = StdioHost;
        for (name, server) in &config.mcp {
            match host.spawn(name, server).await {
                Ok(instance) => {
                    print_catalogue(name, instance.tools());
                    instance.shutdown().await;
                }
                Err(error) => println!("{name}\n  could not spawn: {error}\n"),
            }
        }
    });
    Ok(())
}

/// Print one server's catalogue: a header with its tool count, then each tool's name (aligned) and
/// description. Plain text, so it stays legible piped or redirected.
fn print_catalogue(name: &str, tools: &[McpTool]) {
    let plural = if tools.len() == 1 { "" } else { "s" };
    println!("{name} · {} tool{plural}", tools.len());
    // Align names into a column, but cap the width so one long name does not push every description out.
    let width = tools
        .iter()
        .map(|tool| tool.name.len())
        .max()
        .unwrap_or(0)
        .min(24);
    for tool in tools {
        println!("  {:<width$}  {}", tool.name, tool.description);
    }
    println!();
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

/// Print a response as pretty JSON to stdout — the machine-readable command output a console consumes.
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
    /// The `mcp` introspection command could not run (the async runtime failed to start).
    Mcp(String),
    /// The `events` inspection command could not open or read the event log.
    Events(String),
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
            CliError::Mcp(message) => write!(f, "mcp: {message}"),
            CliError::Events(message) => write!(f, "events: {message}"),
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
            CliError::Mcp(_) => None,
            CliError::Events(_) => None,
        }
    }
}
