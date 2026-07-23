//! The `debug` namespace: diagnostics that read the event log directly (safe while the agent runs, as
//! they take no write lock), plus the belief-arbitration and model-interaction records and the MCP
//! catalogue. These read either the running server (arbitrations, interactions) or the config-selected
//! store and servers (events, brief, revert, mcp), so the dispatch takes both a client and a config.
//!
//! Most commands are read-only, but five write: `revert` and `delete-memory` (documented on their own
//! variants), the two operator corrections `retract` and `clear-occurrence`, and `upgrade-prompts`. The
//! corrections append one forward, operator-sourced event each — a retraction, or an occurrence
//! withdrawal — and `upgrade-prompts` re-registers stale build-default templates (see
//! [`upgrade_prompts`]); each needs the single-writer log lock, so the agent must be stopped first (see
//! [`correction`]).

use clap::Subcommand;
use zuihitsu::config::EnvConfig;

use crate::cli::{client::Client, error::CliError, print_json};

mod brief;
mod correction;
mod delete_memory;
mod embed;
mod events;
mod markdown_fetch;
mod mcp;
mod reindex;
mod revert;
mod upgrade_prompts;

use brief::{BriefSelector, brief};
use events::{EventQuery, events};

#[derive(Subcommand)]
pub(crate) enum DebugCommand {
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
    /// Soft-delete a memory: append a `MemoryDeleted` tombstone so it drops from the graph, search, and
    /// the console on the next fold. Its contents stay in the log (a soft delete preserves history), so
    /// this hides the memory without rewriting the past — appending forward rather than truncating. It
    /// opens the log read-write, so the agent must be stopped first, and it requires `--yes`.
    DeleteMemory {
        /// The memory to delete: its exact name (e.g. `context/console:lua`) or its full id.
        memory: String,
        /// Confirm the soft delete. Without it, the command only reports what it would do.
        #[arg(long)]
        yes: bool,
    },
    /// Retract a content entry to a tombstone, recording why: append an operator-sourced
    /// `EntryRetracted` so it drops from every live surface on the next fold, while its content stays in
    /// the log for audit. Appends forward rather than rewriting history — the fix is itself revertible.
    /// It opens the log read-write, so the agent must be stopped first.
    Retract {
        /// The entry to retract: its full id or a unique prefix of one.
        #[arg(long)]
        entry: String,
        /// Why the entry is being withdrawn. Required — an unaudited retraction is unauditable.
        #[arg(long)]
        reason: String,
    },
    /// Clear a content entry's resolved occurrence: append an operator-sourced `EntryTemporalResolved`
    /// carrying no occurrence, so the entry returns to untimed and any wake-up its occurrence armed is
    /// disarmed on the next fold. Appends forward rather than rewriting history. It opens the log
    /// read-write, so the agent must be stopped first.
    ClearOccurrence {
        /// The entry whose occurrence to clear: its full id or a unique prefix of one.
        #[arg(long)]
        entry: String,
    },
    /// List the recorded model interactions (per-call request, deliberation, tokens, and latency).
    Interactions,
    /// List the recorded belief arbitrations.
    Arbitrations,
    /// List the tools each configured MCP server exposes — spawns the servers directly (no running
    /// agent needed), so you can see a catalogue before narrowing it with `allow`/`deny`.
    Mcp,
    /// Fetch a URL through the real `web.markdown` pipeline — transport, readability extraction, and
    /// Markdown rendering, under the stored web settings — and print the Markdown the agent would
    /// receive. The one debug command that reaches the network.
    MarkdownFetch {
        /// The page URL to fetch (http or https).
        url: String,
        /// Open the SSRF guard for this invocation, so a loopback or private address (a local dev
        /// page) can be fetched without changing the stored settings.
        #[arg(long)]
        allow_private: bool,
    },
    /// Embed two strings and report their cosine similarity — a debug utility for tuning the dedup
    /// and consolidation similarity thresholds.
    Embed {
        /// The first text to compare.
        a: String,
        /// The second text to compare.
        b: String,
    },
    /// Delete the vector index so the next boot rebuilds it from the event log. Used as a
    /// post-upgrade step when the vector schema changes (e.g. the addition of the contextual
    /// embedding space). The agent must be stopped first. Requires `--yes`.
    Reindex {
        /// Confirm the deletion. Without it, the command only reports what it would do.
        #[arg(long)]
        yes: bool,
    },
    /// Re-register stale build-default prompt templates against the running binary's defaults, so an
    /// operator adopts a changed default body without re-running the agent. Default-tracking names
    /// (an unchanged default, Bootstrap-sourced) are upgraded to a newer build default; operator-edited
    /// surfaces are reported as held and overwritten only under `--force`. It opens the log read-write,
    /// so the agent must be stopped first.
    UpgradePrompts {
        /// Overwrite operator-edited (curated) templates too, registering the build default under a
        /// fresh operator registration. Without it, curated names are reported as held and left alone.
        #[arg(long)]
        force: bool,
    },
}

pub(crate) fn dispatch(
    client: &Client,
    config: &EnvConfig,
    command: &DebugCommand,
) -> Result<(), CliError> {
    match command {
        DebugCommand::Events {
            seq,
            from,
            to,
            type_,
            target,
            json,
            summary,
        } => events(
            config,
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
        DebugCommand::Brief { seq, session } => {
            let selector = match (seq, session) {
                (Some(seq), None) => BriefSelector::Seq(*seq),
                (None, Some(session)) => BriefSelector::Session(session.clone()),
                _ => {
                    return Err(CliError::Brief(
                        "pass exactly one of --seq or --session".to_owned(),
                    ));
                }
            };
            brief(config, selector)
        }
        DebugCommand::Revert { seq, yes } => revert::revert(config, *seq, *yes),
        DebugCommand::DeleteMemory { memory, yes } => {
            delete_memory::delete_memory(config, memory, *yes)
        }
        DebugCommand::Retract { entry, reason } => correction::retract(config, entry, reason),
        DebugCommand::ClearOccurrence { entry } => correction::clear_occurrence(config, entry),
        DebugCommand::Interactions => print_json(&client.interactions()?),
        DebugCommand::Arbitrations => print_json(&client.arbitrations()?),
        DebugCommand::Mcp => mcp::mcp(config),
        DebugCommand::MarkdownFetch { url, allow_private } => {
            markdown_fetch::markdown_fetch(config, url, *allow_private)
        }
        DebugCommand::Embed { a, b } => embed::embed(config, a, b),
        DebugCommand::Reindex { yes } => reindex::reindex(config, *yes),
        DebugCommand::UpgradePrompts { force } => upgrade_prompts::upgrade_prompts(config, *force),
    }
}
