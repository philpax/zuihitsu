//! The root subcommand handlers: create the agent, report its status, and read or replace its
//! behavioral settings. These stay at the top level rather than under a namespace.

use std::path::Path;

use zuihitsu::{GenesisStatus, Rollout, SeedSelf};

use crate::cli::{client::Client, error::CliError, print_json};

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

/// Print the agent's current behavioral settings.
pub(crate) fn settings(client: &Client) -> Result<(), CliError> {
    print_json(&client.settings()?)
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
