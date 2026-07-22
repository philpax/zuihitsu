//! The `maintenance` CLI namespace: invoke maintenance passes on demand.

use clap::Subcommand;

use crate::cli::{client::Client, error::CliError};

/// The maintenance subcommands.
#[derive(Subcommand)]
pub(crate) enum MaintenanceCommand {
    /// Run the consolidation pass: cluster semantically-overlapping entries and synthesize
    /// consolidated replacements.
    Consolidate,
    /// Run the canonical-profile pass: give platform stubs readable named identities.
    Canonicalize,
    /// Run the link-redundant entry cleanup pass: retract entries whose content is purely a
    /// description of a link that exists.
    LinkCleanup,
}

pub(crate) fn dispatch(client: &Client, command: &MaintenanceCommand) -> Result<(), CliError> {
    match command {
        MaintenanceCommand::Consolidate => consolidate(client),
        MaintenanceCommand::Canonicalize => canonicalize(client),
        MaintenanceCommand::LinkCleanup => link_cleanup(client),
    }
}

/// Run the consolidation pass.
fn consolidate(client: &Client) -> Result<(), CliError> {
    let count: usize = client.post_no_body("/control/maintenance/consolidate")?;
    tracing::info!(considered = count, "consolidation pass complete");
    Ok(())
}

/// Run the canonical-profile pass.
fn canonicalize(client: &Client) -> Result<(), CliError> {
    let count: usize = client.post_no_body("/control/maintenance/canonicalize")?;
    tracing::info!(considered = count, "canonicalize pass complete");
    Ok(())
}

/// Run the link-redundant entry cleanup pass.
fn link_cleanup(client: &Client) -> Result<(), CliError> {
    let count: usize = client.post_no_body("/control/maintenance/link-cleanup")?;
    tracing::info!(considered = count, "link cleanup pass complete");
    Ok(())
}
