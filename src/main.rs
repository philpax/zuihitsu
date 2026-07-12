//! The zuihitsu binary. Run with no subcommand it boots the long-running HTTP server that hosts the
//! agent (see [`serve`]); with a subcommand it is the operator CLI — a client of that running server
//! (see [`client`]), reaching the agent through its `/control` API rather than opening the store
//! directly (only the server holds the single-writer log lock). A "CLI console": inspection
//! subcommands print their JSON response to stdout; diagnostics go through `tracing` to stderr.

use std::{path::PathBuf, process::ExitCode};

use clap::{Parser, Subcommand};
use tracing_subscriber::{EnvFilter, fmt};
use zuihitsu::config::EnvConfig;

use crate::{
    cli_error::CliError,
    cli_handlers::{create, http_server, mcp, memory, print_json, revert, set_settings, status},
    client::Client,
};

mod cli_brief;
mod cli_error;
mod cli_events;
mod cli_handlers;
mod client;
mod http_server;

use cli_brief::{BriefSelector, brief};
use cli_events::{EventQuery, events};

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
    /// Re-compose a session's contextual brief with the current code and print it beside the brief
    /// frozen at session start (a session's brief is baked into the log, so this is how you see a change
    /// to brief composition against real data without re-running the agent). Reads the log read-only, so
    /// it is safe while the agent is running. Select the session by an event seq it covers, or by its id.
    Brief {
        /// Reproduce the brief of the session active at this event seq (as `events --seq` reports it).
        #[arg(long)]
        seq: Option<u64>,
        /// Reproduce the brief of the session with this id, or a unique prefix of it.
        #[arg(long)]
        session: Option<String>,
    },
    /// Revert the agent to a prior event: truncate the log past `seq`, then reset the derived stores so
    /// the next boot rebuilds at that point. Destructive and irreversible. It opens the log read-write,
    /// so the agent must be stopped first, and it requires `--yes` to proceed.
    Revert {
        /// The sequence number to revert to. Every event after it is removed.
        #[arg(long)]
        seq: u64,
        /// Confirm the destructive truncation. Without it, the command only reports what it would do.
        #[arg(long)]
        yes: bool,
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
        Command::Brief { seq, session } => {
            let selector = match (seq, session) {
                (Some(seq), None) => BriefSelector::Seq(*seq),
                (None, Some(session)) => BriefSelector::Session(session.clone()),
                _ => {
                    return Err(CliError::Brief(
                        "pass exactly one of --seq or --session".to_owned(),
                    ));
                }
            };
            brief(&config, selector)
        }
        Command::Revert { seq, yes } => revert(&config, *seq, *yes),
    }
}
