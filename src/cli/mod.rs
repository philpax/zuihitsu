//! The operator CLI's command handlers, grouped by namespace: [`interact`] drives the agent, [`state`]
//! inspects its materialized memory, and [`debug`] reads the event log directly. [`root`] holds the
//! root-level commands (create, status, settings). [`client`] and [`error`] are the shared transport
//! and error types. The parser and dispatch live in the binary root (`main.rs`); this module owns the
//! handlers those routes call, and the one shared output helper.

use serde::Serialize;

use crate::cli::error::CliError;

pub(crate) mod client;
pub(crate) mod debug;
pub(crate) mod error;
pub(crate) mod interact;
pub(crate) mod maintenance;
pub(crate) mod root;
pub(crate) mod state;

/// Print a response as pretty JSON to stdout — the machine-readable command output a console consumes.
pub(crate) fn print_json<T: Serialize>(value: &T) -> Result<(), CliError> {
    let json = serde_json::to_string_pretty(value).map_err(CliError::Render)?;
    println!("{json}");
    Ok(())
}
