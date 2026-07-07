//! A CLI-level failure, naming the operation and the resource it was acting on.

use std::path::PathBuf;

use zuihitsu::ConfigError;

use crate::client::ClientError;
use crate::http_server;

#[derive(Debug)]
pub(crate) enum CliError {
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
