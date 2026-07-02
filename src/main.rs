//! The zuihitsu binary. Run with no subcommand it boots the long-running HTTP server that hosts the
//! agent (see [`serve`]); with a subcommand it is the operator CLI — a client of that running server
//! (see [`client`]), reaching the agent through its `/control` API rather than opening the store
//! directly (only the server holds the single-writer log lock). A "CLI console": inspection
//! subcommands print their JSON response to stdout; diagnostics go through `tracing` to stderr.

use std::{
    collections::BTreeMap,
    io::Write,
    path::{Path, PathBuf},
    process::ExitCode,
};

use anstyle::{AnsiColor, Style};
use clap::{Parser, Subcommand};
use serde::Serialize;
use tracing_subscriber::{EnvFilter, fmt};
#[cfg(test)]
use zuihitsu::Volatility;
use zuihitsu::{
    ConfigError, Event, GenesisStatus, McpHost, McpTool, MemoryId, Rollout, SeedSelf, Seq,
    SqliteStore, StdioHost, Store,
    config::EnvConfig,
    event::{EventPayload, Teller, TerminalCause, TurnRole, Visibility},
};

use crate::client::{Client, ClientError};

mod client;
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
        Command::Revert { seq, yes } => revert(&config, *seq, *yes),
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

    // Resolve memory ids to names from the create/rename events across the WHOLE log, before the
    // filters narrow the view — so a link or append in a slice still reads in names even when the
    // memory was created outside the slice. The log is self-describing, so no graph read is needed.
    let names = name_map(&events);

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
    } else if json {
        // Clean NDJSON: one whole event per line (seq, recorded_at, payload), no columns, so the
        // output pipes straight into `jq` or any line-oriented parser.
        for event in &events {
            let line = serde_json::to_string(event).map_err(|source| {
                CliError::Events(format!(
                    "could not serialize event {}: {source}",
                    event.seq.0
                ))
            })?;
            println!("{line}");
        }
    } else {
        let mut out = anstream::stdout();
        for event in &events {
            let _ = write_event(&mut out, event, &names, false);
        }
    }
    Ok(())
}

/// Write one event as the two-line listing — a dim seq, the kind in its category color, then the
/// payload gloss — to `out`. When `faded`, the whole event is dimmed instead of colored. The shared
/// renderer for the `events` listing (never faded) and the `revert` preview (which fades the events it
/// would remove). `anstream::stdout` adapts the styling to the destination — colored on a terminal
/// (including Windows), stripped when piped, off under `NO_COLOR` — so there is no manual terminal
/// gate; an `anstyle::Style` renders its prefix as `{style}` and its reset as `{style:#}`.
fn write_event(
    out: &mut impl Write,
    event: &Event,
    names: &BTreeMap<String, String>,
    faded: bool,
) -> std::io::Result<()> {
    let dim = Style::new().dimmed();
    let kind_style = if faded {
        dim
    } else {
        Style::new().fg_color(Some(category_color(&event.payload).into()))
    };
    let kind = event.payload.kind();
    writeln!(
        out,
        "{dim}{:>6}{dim:#}  {kind_style}{kind}{kind_style:#}",
        event.seq.0
    )?;
    let detail = describe_event(&event.payload, names);
    if !detail.is_empty() {
        if faded {
            writeln!(out, "        {dim}{detail}{dim:#}")?;
        } else {
            writeln!(out, "        {detail}")?;
        }
    }
    Ok(())
}

/// The latest name of every memory the log creates or renames, keyed by its id string — the lookup
/// `describe_event` uses to render an event in names instead of ULIDs.
fn name_map(events: &[Event]) -> BTreeMap<String, String> {
    let mut names = BTreeMap::new();
    for event in events {
        match &event.payload {
            EventPayload::MemoryCreated { id, name } => {
                names.insert(id.0.to_string(), name.as_str().to_owned());
            }
            EventPayload::MemoryRenamed { id, new_name, .. } => {
                names.insert(id.0.to_string(), new_name.as_str().to_owned());
            }
            _ => {}
        }
    }
    names
}

/// A human-readable one-line gloss of an event — what happened, with memory ids resolved to names —
/// so the timeline reads without decoding JSON, the way the console viewer renders it. Falls back to
/// the bare event kind for the structural events with no salient content to show.
fn describe_event(payload: &EventPayload, names: &BTreeMap<String, String>) -> String {
    let name = |id: &MemoryId| {
        names
            .get(&id.0.to_string())
            .cloned()
            .unwrap_or_else(|| short_id(&id.0.to_string()))
    };
    match payload {
        EventPayload::ConversationTurn {
            role,
            text,
            participant,
            ..
        } => {
            let who = match (role, participant) {
                (TurnRole::Agent, _) => "agent".to_owned(),
                (TurnRole::System, _) => "system".to_owned(),
                (TurnRole::Participant, Some(id)) => name(id),
                (TurnRole::Participant, None) => "participant".to_owned(),
            };
            format!("«{who}» {}", oneline(text, 90))
        }
        EventPayload::MemoryContentAppended {
            id,
            text,
            visibility,
            told_by,
            ..
        } => format!(
            "{} ← \"{}\"  [{}, {}]",
            name(id),
            oneline(text, 60),
            visibility_label(visibility),
            teller_label(told_by, &name),
        ),
        EventPayload::LuaExecuted {
            result,
            terminal_cause,
            ..
        } => match (terminal_cause, result) {
            (Some(TerminalCause::Error(message)), _) => format!("error: {}", oneline(message, 80)),
            (Some(TerminalCause::Aborted(reason)), _) => {
                format!("aborted: {}", oneline(reason, 80))
            }
            (None, Some(result)) => oneline(result, 100),
            (None, None) => String::new(),
        },
        EventPayload::MemoryCreated { name: created, .. } => {
            format!("created {}", created.as_str())
        }
        EventPayload::MemoryRenamed {
            old_name, new_name, ..
        } => format!("renamed {} → {}", old_name.as_str(), new_name.as_str()),
        EventPayload::MemoryDeleted { id } => format!("deleted {}", name(id)),
        EventPayload::MemorySuperseded { id, .. } => format!("{}: superseded an entry", name(id)),
        EventPayload::MemoryDescriptionRegenerated { id, .. } => {
            format!("{}: re-described", name(id))
        }
        EventPayload::BeliefArbitrated {
            memory,
            competing_entries,
            ..
        } => format!(
            "{}: arbitrated ({} competing)",
            name(memory),
            competing_entries.len()
        ),
        EventPayload::MemoryVolatilitySet { id, volatility } => {
            format!("{}: volatility {}", name(id), volatility)
        }
        EventPayload::LinkCreated {
            from, to, relation, ..
        } => format!("{} —{}→ {}", name(from), relation.as_str(), name(to)),
        EventPayload::LinkRemoved {
            from, to, relation, ..
        } => format!("{} —{}✗ {}", name(from), relation.as_str(), name(to)),
        EventPayload::TagAppliedToMemory { memory, tag } => {
            format!("{} +#{}", name(memory), tag.as_str())
        }
        EventPayload::TagRemovedFromMemory { memory, tag } => {
            format!("{} −#{}", name(memory), tag.as_str())
        }
        EventPayload::TagCreated { name: tag, .. } => format!("created #{}", tag.as_str()),
        EventPayload::LinkTypeRegistered {
            name: rel, inverse, ..
        } => {
            format!("registered relation {}/{}", rel.as_str(), inverse.as_str())
        }
        EventPayload::ScheduledJobFired { memory, .. } => {
            format!("wake-up fired ({})", name(memory))
        }
        EventPayload::ScheduledItemSurfaced { memory, .. } => {
            format!("wake-up surfaced ({})", name(memory))
        }
        EventPayload::EntryTemporalResolved { id, .. } => format!("{}: resolved a date", name(id)),
        EventPayload::EntryTemporalResolveFailed { id, .. } => {
            format!("{}: failed to resolve a date", name(id))
        }
        EventPayload::MergeProposed { from, to } => {
            format!("merge proposed: {} → {}", name(from), name(to))
        }
        EventPayload::MergeAdjudicated {
            from, to, accepted, ..
        } => format!(
            "merge {}: {} → {}",
            if *accepted { "accepted" } else { "refused" },
            name(from),
            name(to)
        ),
        EventPayload::ModelCalled { phase, .. } => format!("{phase:?}"),
        EventPayload::EmbeddingModelChanged { from, to } => {
            format!("embedding model {from} → {to}")
        }
        EventPayload::GenesisCompleted {
            manifest_hash,
            template_versions,
        } => format!(
            "manifest {}…, {} templates",
            &manifest_hash[..manifest_hash.len().min(10)],
            template_versions.len()
        ),
        EventPayload::PromptTemplateRegistered {
            name: template,
            version,
            ..
        } => format!("{template:?} v{version}"),
        EventPayload::TagDescriptionChanged {
            name: tag,
            new_description,
        } => format!("#{}: \"{}\"", tag.as_str(), oneline(new_description, 60)),
        EventPayload::ConfigSet { .. } => "settings updated".to_owned(),
        // The remaining events — session and conversation boundaries, soft deletes — are markers whose
        // kind on the first line is the whole story; they get no second line.
        _ => String::new(),
    }
}

/// Collapse whitespace runs (so a multi-line script or reply stays one line) and clip to `max` chars.
fn oneline(text: &str, max: usize) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() > max {
        let kept: String = collapsed.chars().take(max).collect();
        format!("{kept}…")
    } else {
        collapsed
    }
}

/// The last segment of a ULID, for a memory the log never names (it shows enough to disambiguate).
fn short_id(id: &str) -> String {
    format!("…{}", &id[id.len().saturating_sub(6)..])
}

fn visibility_label(visibility: &Visibility) -> &'static str {
    match visibility {
        Visibility::Public => "public",
        Visibility::Attributed => "attributed",
        Visibility::PrivateToTeller => "private",
        Visibility::Exclude(_) => "excluded",
    }
}

fn teller_label(teller: &Teller, name: &impl Fn(&MemoryId) -> String) -> String {
    match teller {
        Teller::Participant(id) => name(id),
        Teller::Agent => "agent".to_owned(),
        Teller::Bootstrap => "seed".to_owned(),
    }
}

/// The color an event's kind is rendered in, grouped so a glance separates conversation from writes
/// from relations from belief from telemetry. Matched on the variant (not its name), so adding an
/// event kind is a compile error here until it is given a category.
fn category_color(payload: &EventPayload) -> AnsiColor {
    match payload {
        // Conversation and session flow.
        EventPayload::ConversationTurn { .. }
        | EventPayload::ConversationStarted { .. }
        | EventPayload::ConversationEnded { .. }
        | EventPayload::SessionStarted { .. }
        | EventPayload::SessionEnded { .. }
        | EventPayload::ParticipantJoined { .. } => AnsiColor::Cyan,
        // A recorded fact — the substance of the log.
        EventPayload::MemoryContentAppended { .. } => AnsiColor::Green,
        // Memory structure and the entry lifecycle.
        EventPayload::MemoryCreated { .. }
        | EventPayload::MemoryRenamed { .. }
        | EventPayload::MemoryDeleted { .. }
        | EventPayload::MemorySuperseded { .. }
        | EventPayload::MemoryDescriptionRegenerated { .. }
        | EventPayload::MemoryVolatilitySet { .. }
        | EventPayload::EntryTemporalResolved { .. }
        | EventPayload::EntryTemporalResolveFailed { .. } => AnsiColor::BrightGreen,
        // Relations and cross-platform identity.
        EventPayload::LinkCreated { .. }
        | EventPayload::LinkRemoved { .. }
        | EventPayload::LinkTypeRegistered { .. }
        | EventPayload::ParticipantIdentified { .. } => AnsiColor::Blue,
        // Tags and scheduling.
        EventPayload::TagCreated { .. }
        | EventPayload::TagDescriptionChanged { .. }
        | EventPayload::TagAppliedToMemory { .. }
        | EventPayload::TagRemovedFromMemory { .. }
        | EventPayload::ScheduledJobFired { .. }
        | EventPayload::ScheduledItemSurfaced { .. } => AnsiColor::Yellow,
        // Belief arbitration and merges.
        EventPayload::BeliefArbitrated { .. }
        | EventPayload::MergeProposed { .. }
        | EventPayload::MergeAdjudicated { .. }
        | EventPayload::LinksInferred { .. }
        | EventPayload::DescribePassCompleted { .. } => AnsiColor::Magenta,
        // Telemetry and structural or config events — the quiet background.
        EventPayload::ModelCalled { .. }
        | EventPayload::LuaExecuted { .. }
        | EventPayload::GenesisCompleted { .. }
        | EventPayload::ConfigSet { .. }
        | EventPayload::PromptTemplateRegistered { .. }
        | EventPayload::EmbeddingModelChanged { .. } => AnsiColor::BrightBlack,
    }
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
fn http_server(config_path: &Path) -> Result<(), CliError> {
    crate::http_server::run_blocking(config_path).map_err(CliError::HttpServer)
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

/// Revert the agent to a prior event. Opens the log read-write — which fails if the agent holds the
/// write lock, so a running agent is refused — and truncates every event past `to`. The materialized
/// graph and the vector index only roll forward, so they cannot be walked back; instead the graph and
/// vector files are dropped (the next boot replays and re-embeds from the shortened log) and any
/// snapshot past `to` is discarded so `restore_if_stale` cannot copy a future state back. Without
/// `--yes`, it reports what it would do and changes nothing.
/// Print the window of events around a revert point — up to `CONTEXT` on each side — to stdout, with
/// the events that would be removed (those after `to`) greyed out, and a marked rule between the kept
/// head and the removed tail. Sharing `write_event` with the `events` listing, so the preview reads
/// the same. The total removed count is named even when the window shows only part of the tail.
fn show_revert_preview(events: &[Event], names: &BTreeMap<String, String>, to: Seq, head: Seq) {
    const CONTEXT: u64 = 8;
    let mut out = anstream::stdout();
    let removed_total = head.0 - to.0;
    let (lo, hi) = (to.0.saturating_sub(CONTEXT), to.0.saturating_add(CONTEXT));

    let mut rule_printed = false;
    let mut shown_removed = 0u64;
    for event in events.iter().filter(|e| e.seq.0 >= lo && e.seq.0 <= hi) {
        let faded = event.seq.0 > to.0;
        if faded && !rule_printed {
            print_revert_rule(&mut out, to, removed_total);
            rule_printed = true;
        }
        let _ = write_event(&mut out, event, names, faded);
        shown_removed += u64::from(faded);
    }
    if !rule_printed {
        print_revert_rule(&mut out, to, removed_total);
    }
    if removed_total > shown_removed {
        let dim = Style::new().dimmed();
        let _ = writeln!(
            out,
            "        {dim}… and {} more removed{dim:#}",
            removed_total - shown_removed
        );
    }
}

/// The marked rule between the kept head and the removed tail in the revert preview.
fn print_revert_rule(out: &mut impl Write, to: Seq, removed_total: u64) {
    let mark = Style::new().fg_color(Some(AnsiColor::Yellow.into())).bold();
    let _ = writeln!(
        out,
        "{mark}      ── revert here · seq {} becomes the new head · {removed_total} event(s) below are removed ──{mark:#}",
        to.0,
    );
}

fn revert(config: &EnvConfig, to: u64, yes: bool) -> Result<(), CliError> {
    let to = Seq(to);
    let log_path = config.storage.event_log();
    let mut store = SqliteStore::open(&log_path).map_err(|source| {
        CliError::Revert(format!(
            "could not open the event log at {} for writing (is the agent running?): {source}",
            log_path.display()
        ))
    })?;
    let head = store
        .head()
        .map_err(|source| CliError::Revert(format!("could not read the log head: {source}")))?;
    if to >= head {
        return Err(CliError::Revert(format!(
            "seq {} is at or past the current head {}; nothing to revert",
            to.0, head.0
        )));
    }

    // Preview the cut: the window of events around the revert point, with everything after it greyed
    // out and a marked rule between, so it is unmistakable what survives and what is removed — shown
    // before anything is touched, in both the dry run and the confirmed run.
    let events = store
        .read_from(Seq(0))
        .map_err(|source| CliError::Revert(format!("could not read the log: {source}")))?;
    let names = name_map(&events);
    show_revert_preview(&events, &names, to, head);

    if !yes {
        tracing::warn!(
            "re-run with --yes to confirm reverting to seq {} (removes {} events and rebuilds the \
             graph and vector index)",
            to.0,
            head.0 - to.0,
        );
        return Ok(());
    }

    let removed = store
        .truncate_to(to)
        .map_err(|source| CliError::Revert(format!("could not truncate the log: {source}")))?;
    drop(store); // release the write lock before touching the derived files.

    let graph_path = config.storage.graph();
    let vectors_path = config.storage.vectors();
    let snapshot_dir = config.snapshots.effective_dir(&graph_path);
    remove_db(&graph_path)?;
    remove_db(&vectors_path)?;
    let pruned = zuihitsu::snapshot::discard_after(&snapshot_dir, to).map_err(|source| {
        CliError::Revert(format!(
            "could not discard snapshots past the revert point: {source}"
        ))
    })?;

    tracing::info!(
        "reverted to seq {}: removed {removed} event(s) and {} snapshot(s); the next boot rebuilds \
         the graph and re-embeds the vector index from the shortened log",
        to.0,
        pruned.len(),
    );
    Ok(())
}

/// Remove a SQLite database file and its `-wal`/`-shm` sidecars, treating an absent file as success.
/// Dropping the derived graph and vector stores so the next boot rebuilds them from the log.
fn remove_db(path: &Path) -> Result<(), CliError> {
    for suffix in ["", "-wal", "-shm"] {
        let mut target = path.as_os_str().to_owned();
        target.push(suffix);
        let target = PathBuf::from(target);
        match std::fs::remove_file(&target) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(CliError::Revert(format!(
                    "could not remove {}: {source}",
                    target.display()
                )));
            }
        }
    }
    Ok(())
}

/// A CLI-level failure, naming the operation and the resource it was acting on.
#[derive(Debug)]
enum CliError {
    LoadConfig {
        source: ConfigError,
    },
    HttpServer(http_server::ServeError),
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
    /// The `revert` command could not truncate the log or reset the derived stores.
    Revert(String),
}

impl From<ClientError> for CliError {
    fn from(error: ClientError) -> Self {
        CliError::Client(error)
    }
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CliError::LoadConfig { source } => {
                write!(f, "could not load the config: {source}")
            }
            CliError::HttpServer(source) => {
                write!(f, "the HTTP server exited with an error: {source}")
            }
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
            CliError::Revert(message) => write!(f, "revert: {message}"),
        }
    }
}

impl std::error::Error for CliError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            CliError::LoadConfig { source } => Some(source),
            CliError::HttpServer(source) => Some(source),
            CliError::Client(source) => Some(source),
            CliError::ReadFile { source, .. } => Some(source),
            CliError::ParseSettings { source, .. } => Some(source),
            CliError::Render(source) => Some(source),
            CliError::Mcp(_) => None,
            CliError::Events(_) | CliError::Revert(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zuihitsu::{ids::MemoryName, time::Timestamp, vocabulary::TagName};

    fn ev(seq: u64, payload: EventPayload) -> Event {
        Event {
            seq: Seq(seq),
            recorded_at: Timestamp::from_millis(0),
            payload,
        }
    }

    #[test]
    fn name_map_resolves_a_create_and_the_latest_rename() {
        let id = MemoryId::generate();
        let events = vec![
            ev(
                1,
                EventPayload::memory_created(id, MemoryName::new("person/dave")),
            ),
            ev(
                2,
                EventPayload::memory_renamed(
                    id,
                    MemoryName::new("person/dave"),
                    MemoryName::new("person/sarah"),
                ),
            ),
        ];
        let names = name_map(&events);
        assert_eq!(
            names.get(&id.0.to_string()).map(String::as_str),
            Some("person/sarah")
        );
    }

    #[test]
    fn describe_event_glosses_payloads_resolving_ids_to_names() {
        let id = MemoryId::generate();
        let names = name_map(&[ev(
            1,
            EventPayload::memory_created(id, MemoryName::new("person/dave")),
        )]);

        assert_eq!(
            describe_event(
                &EventPayload::memory_created(id, MemoryName::new("person/dave")),
                &names
            ),
            "created person/dave"
        );
        assert_eq!(
            describe_event(
                &EventPayload::memory_volatility_set(id, Volatility::High),
                &names
            ),
            "person/dave: volatility high"
        );
        assert_eq!(
            describe_event(
                &EventPayload::tag_applied_to_memory(id, TagName::new("hobbies")),
                &names
            ),
            "person/dave +#hobbies"
        );
        // A memory the log never names falls back to a short id rather than panicking.
        let other = MemoryId::generate();
        assert!(
            describe_event(&EventPayload::memory_deleted(other), &names).starts_with("deleted …")
        );
    }

    #[test]
    fn category_color_groups_by_kind() {
        let id = MemoryId::generate();
        assert_eq!(
            category_color(&EventPayload::memory_created(id, MemoryName::new("self"))),
            AnsiColor::BrightGreen
        );
        assert_eq!(
            category_color(&EventPayload::tag_created(TagName::new("x"), "d")),
            AnsiColor::Yellow
        );
        assert_eq!(
            category_color(&EventPayload::memory_deleted(id)),
            AnsiColor::BrightGreen
        );
    }

    #[test]
    fn json_listing_is_ndjson_that_round_trips() {
        let event = ev(
            7,
            EventPayload::memory_created(MemoryId::generate(), MemoryName::new("person/dave")),
        );
        let line = serde_json::to_string(&event).unwrap();
        assert!(!line.contains('\n'), "one event must serialize to one line");
        let parsed: Event = serde_json::from_str(&line).unwrap();
        assert_eq!(parsed.seq, Seq(7));
        assert_eq!(parsed.payload.kind(), "MemoryCreated");
    }
}
