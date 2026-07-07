//! A failure starting or running the server.

use std::{io, net::SocketAddr, path::PathBuf};

use zuihitsu::{
    ConfigError, GraphError, ServerError, StoreError, VectorError, snapshot::SnapshotError,
};

/// A failure starting or running the server.
#[derive(Debug)]
pub enum ServeError {
    Config(ConfigError),
    Runtime(io::Error),
    CreateDir {
        path: PathBuf,
        source: io::Error,
    },
    OpenEventLog {
        path: PathBuf,
        source: StoreError,
    },
    OpenGraph {
        path: PathBuf,
        source: GraphError,
    },
    OpenVectors {
        path: PathBuf,
        source: VectorError,
    },
    /// Restoring the graph from a snapshot at boot failed (spec §Snapshots).
    Snapshot(SnapshotError),
    /// A server operation (boot, reading settings, connecting MCP) failed at startup. Boxed because
    /// `ServerError` (= `InstanceError`) transitively owns `TurnError`/`LuaError` and is large enough
    /// to push `CliError` past the `result_large_err` lint threshold.
    Server(Box<ServerError>),
    /// A model endpoint is configured but `[model] context_length` is not — the API cannot report the
    /// window, so the operator must state it (the agent's compaction budget derives from it).
    MissingContextLength,
    Bind {
        addr: SocketAddr,
        source: io::Error,
    },
    Serve(io::Error),
}

impl From<ServerError> for ServeError {
    fn from(error: ServerError) -> Self {
        ServeError::Server(Box::new(error))
    }
}

impl std::fmt::Display for ServeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ServeError::Config(source) => write!(f, "serve: could not load config: {source}"),
            ServeError::Runtime(source) => {
                write!(f, "serve: could not start the runtime: {source}")
            }
            ServeError::CreateDir { path, source } => {
                write!(f, "serve: could not create {}: {source}", path.display())
            }
            ServeError::OpenEventLog { path, source } => {
                write!(
                    f,
                    "serve: could not open the event log at {}: {source}",
                    path.display()
                )
            }
            ServeError::OpenGraph { path, source } => {
                write!(
                    f,
                    "serve: could not open the graph at {}: {source}",
                    path.display()
                )
            }
            ServeError::OpenVectors { path, source } => {
                write!(
                    f,
                    "serve: could not open the vector index at {}: {source}",
                    path.display()
                )
            }
            ServeError::Snapshot(source) => {
                write!(
                    f,
                    "serve: could not restore the graph from a snapshot: {source}"
                )
            }
            ServeError::Server(source) => write!(f, "serve: {source}"),
            ServeError::MissingContextLength => write!(
                f,
                "serve: a model endpoint is configured but [model] context_length is not set — \
                 state your model's context window in tokens (the API does not report it)"
            ),
            ServeError::Bind { addr, source } => {
                write!(f, "serve: could not bind {addr}: {source}")
            }
            ServeError::Serve(source) => write!(f, "serve: the HTTP server failed: {source}"),
        }
    }
}

impl std::error::Error for ServeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ServeError::Config(source) => Some(source),
            ServeError::Runtime(source) => Some(source),
            ServeError::CreateDir { source, .. } => Some(source),
            ServeError::OpenEventLog { source, .. } => Some(source),
            ServeError::OpenGraph { source, .. } => Some(source),
            ServeError::OpenVectors { source, .. } => Some(source),
            ServeError::Snapshot(source) => Some(source),
            ServeError::Server(source) => Some(source.as_ref()),
            ServeError::MissingContextLength => None,
            ServeError::Bind { source, .. } => Some(source),
            ServeError::Serve(source) => Some(source),
        }
    }
}
