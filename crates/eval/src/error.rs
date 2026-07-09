//! The harness error type — manual `Display` with a `eval:` context prefix, per the project
//! convention (no `thiserror`).

use std::path::PathBuf;

use zuihitsu::{ConfigError, GraphError, ModelError, ServerError, VectorError};

#[derive(Debug)]
pub enum EvalError {
    LoadConfig {
        path: PathBuf,
        source: Box<ConfigError>,
    },
    Graph(Box<GraphError>),
    Server(Box<ServerError>),
    Model(Box<ModelError>),
    Vector(Box<VectorError>),
    /// The judge model did not return a parseable verdict.
    Judge(String),
    WriteOutput {
        path: PathBuf,
        source: std::io::Error,
    },
    /// A `--resume` sidecar could not be folded back (malformed or missing its manifest).
    ResumeSidecar {
        path: PathBuf,
        reason: String,
    },
    /// The `--serve` live endpoint could not bind or serve.
    Serve(std::io::Error),
    Serialize(Box<serde_json::Error>),
    /// The `--name` is not a bare filename (empty, or carries a path separator or `..`).
    BadName(String),
    /// A step's run-time-dependent resolution failed at execution: a `ConfirmProposedMerge` that
    /// required a proposal found none, or a `StepText::WithTurnRef` referenced no recorded turn.
    Executor(String),
    /// `analyze`: the package file could not be read.
    ReadPackage {
        path: PathBuf,
        source: std::io::Error,
    },
    /// `analyze`: the package file is not a valid eval package.
    LoadPackage {
        path: PathBuf,
        source: Box<serde_json::Error>,
    },
    /// `events`: a scenario/run could not be resolved, or the package could not be rendered.
    Events(String),
    /// `replay`: an argument was invalid, a scenario/run could not be resolved, the recorded run
    /// drifted from the current script, or the restore could not be assembled.
    Replay(String),
}

impl std::fmt::Display for EvalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EvalError::LoadConfig { path, source } => {
                write!(
                    f,
                    "eval: could not load config at {}: {source}",
                    path.display()
                )
            }
            EvalError::Graph(source) => write!(f, "eval: graph: {source}"),
            EvalError::Server(source) => write!(f, "eval: {source}"),
            EvalError::Model(source) => write!(f, "eval: model: {source}"),
            EvalError::Vector(source) => write!(f, "eval: vector index: {source}"),
            EvalError::Judge(message) => write!(f, "eval: judge: {message}"),
            EvalError::WriteOutput { path, source } => {
                write!(f, "eval: could not write {}: {source}", path.display())
            }
            EvalError::ResumeSidecar { path, reason } => {
                write!(
                    f,
                    "eval: could not resume from {}: {reason}",
                    path.display()
                )
            }
            EvalError::Serve(source) => write!(f, "eval: live serve: {source}"),
            EvalError::Serialize(source) => {
                write!(f, "eval: could not serialize the package: {source}")
            }
            EvalError::BadName(name) => {
                write!(
                    f,
                    "eval: --name must be a bare filename (no path separators or `..`), got {name:?}"
                )
            }
            EvalError::Executor(message) => write!(f, "eval: executor: {message}"),
            EvalError::ReadPackage { path, source } => {
                write!(
                    f,
                    "eval: could not read the package at {}: {source}",
                    path.display()
                )
            }
            EvalError::LoadPackage { path, source } => {
                write!(
                    f,
                    "eval: {} is not a valid eval package: {source}",
                    path.display()
                )
            }
            EvalError::Events(message) => write!(f, "eval: events: {message}"),
            EvalError::Replay(message) => write!(f, "eval: replay: {message}"),
        }
    }
}

impl std::error::Error for EvalError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            EvalError::LoadConfig { source, .. } => Some(source.as_ref()),
            EvalError::Graph(source) => Some(source.as_ref()),
            EvalError::Server(source) => Some(source.as_ref()),
            EvalError::Model(source) => Some(source.as_ref()),
            EvalError::Vector(source) => Some(source.as_ref()),
            EvalError::WriteOutput { source, .. } => Some(source),
            EvalError::Serialize(source) => Some(source.as_ref()),
            EvalError::Serve(source) => Some(source),
            EvalError::ReadPackage { source, .. } => Some(source),
            EvalError::LoadPackage { source, .. } => Some(source.as_ref()),
            EvalError::Judge(_)
            | EvalError::ResumeSidecar { .. }
            | EvalError::BadName(_)
            | EvalError::Executor(_)
            | EvalError::Events(_)
            | EvalError::Replay(_) => None,
        }
    }
}

impl From<GraphError> for EvalError {
    fn from(source: GraphError) -> EvalError {
        EvalError::Graph(Box::new(source))
    }
}

impl From<ServerError> for EvalError {
    fn from(source: ServerError) -> EvalError {
        EvalError::Server(Box::new(source))
    }
}

impl From<ModelError> for EvalError {
    fn from(source: ModelError) -> EvalError {
        EvalError::Model(Box::new(source))
    }
}

impl From<VectorError> for EvalError {
    fn from(source: VectorError) -> EvalError {
        EvalError::Vector(Box::new(source))
    }
}

impl From<serde_json::Error> for EvalError {
    fn from(source: serde_json::Error) -> EvalError {
        EvalError::Serialize(Box::new(source))
    }
}
