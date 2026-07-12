//! The `state` namespace: inspect the agent's materialized state — a memory or a namespace of them,
//! a memory's entries, a conversation's sessions, the recurring occurrences, and the snapshot control.

use clap::Subcommand;

use crate::cli::{client::Client, error::CliError, print_json};

#[derive(Subcommand)]
pub(crate) enum StateCommand {
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
    /// Write a graph snapshot now (a checkpoint to speed the next cold boot, or before an experiment).
    Snapshot,
}

pub(crate) fn dispatch(client: &Client, command: &StateCommand) -> Result<(), CliError> {
    match command {
        StateCommand::Memory { name } => memory(client, name),
        StateCommand::Memories { prefix } => print_json(&client.memories(prefix)?),
        StateCommand::Entries { name } => print_json(&client.entries(name)?),
        StateCommand::Sessions { platform, scope } => {
            print_json(&client.sessions(platform, scope)?)
        }
        StateCommand::Recurring => print_json(&client.recurring()?),
        StateCommand::Snapshot => print_json(&client.snapshot()?),
    }
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
