//! The CLI subcommand handlers — inspection, mutation, and revert commands.

use std::{
    collections::BTreeMap,
    io::Write,
    path::{Path, PathBuf},
};

use anstyle::{AnsiColor, Style};
use serde::Serialize;
use zuihitsu::{
    Event, GenesisStatus, McpHost, McpTool, Rollout, SeedSelf, Seq, SqliteStore, Store,
    config::EnvConfig,
};

use crate::{
    cli_error::CliError,
    cli_events::{name_map, write_event},
    client::Client,
};

/// List the tools each configured MCP server exposes. Spawns the servers directly over stdio (no
/// running agent needed), snapshots each catalogue, and prints it as a readable listing — a server
/// that cannot be brought up reports its error and the rest still run, so one missing binary does not
/// hide the others. The operator reads this to choose an `allow`/`deny` projection.
pub(crate) fn mcp(config: &EnvConfig) -> Result<(), CliError> {
    if config.mcp.is_empty() {
        tracing::info!("no MCP servers configured; add an [mcp.<name>] block to the config");
        return Ok(());
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|source| CliError::Mcp(format!("could not start the async runtime: {source}")))?;
    runtime.block_on(async {
        let host = zuihitsu::StdioHost;
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
pub(crate) fn http_server(config_path: &Path) -> Result<(), CliError> {
    crate::http_server::run_blocking(config_path).map_err(CliError::HttpServer)
}

pub(crate) fn create(
    client: &Client,
    name: &str,
    persona: &str,
    seed: &[String],
) -> Result<(), CliError> {
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

pub(crate) fn status(client: &Client) -> Result<(), CliError> {
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

pub(crate) fn memory(client: &Client, name: &str) -> Result<(), CliError> {
    match client.memory(name)? {
        Some(view) => print_json(&view),
        None => {
            tracing::info!(%name, "no such memory");
            Ok(())
        }
    }
}

pub(crate) fn set_settings(client: &Client, file: &Path) -> Result<(), CliError> {
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
pub(crate) fn print_json<T: Serialize>(value: &T) -> Result<(), CliError> {
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
pub(crate) fn show_revert_preview(
    events: &[Event],
    names: &BTreeMap<String, String>,
    to: Seq,
    head: Seq,
) {
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

pub(crate) fn revert(config: &EnvConfig, to: u64, yes: bool) -> Result<(), CliError> {
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
