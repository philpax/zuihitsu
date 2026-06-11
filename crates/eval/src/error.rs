//! The harness error type — manual `Display` with a `eval:` context prefix, per the project
//! convention (no `thiserror`).

use std::path::PathBuf;

use zuihitsu::{ConfigError, GraphError, ModelError, ServerError, VectorError};

#[derive(Debug)]
pub enum EvalError {
    LoadConfig {
        path: PathBuf,
        source: ConfigError,
    },
    Graph(GraphError),
    Server(ServerError),
    Model(ModelError),
    Vector(VectorError),
    /// The judge model did not return a parseable verdict.
    Judge(String),
    WriteOutput {
        path: PathBuf,
        source: std::io::Error,
    },
    Serialize(serde_json::Error),
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
            EvalError::Serialize(source) => {
                write!(f, "eval: could not serialize the package: {source}")
            }
        }
    }
}

impl std::error::Error for EvalError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            EvalError::LoadConfig { source, .. } => Some(source),
            EvalError::Graph(source) => Some(source),
            EvalError::Server(source) => Some(source),
            EvalError::Model(source) => Some(source),
            EvalError::Vector(source) => Some(source),
            EvalError::WriteOutput { source, .. } => Some(source),
            EvalError::Serialize(source) => Some(source),
            EvalError::Judge(_) => None,
        }
    }
}

impl From<GraphError> for EvalError {
    fn from(source: GraphError) -> EvalError {
        EvalError::Graph(source)
    }
}

impl From<ServerError> for EvalError {
    fn from(source: ServerError) -> EvalError {
        EvalError::Server(source)
    }
}

impl From<ModelError> for EvalError {
    fn from(source: ModelError) -> EvalError {
        EvalError::Model(source)
    }
}

impl From<VectorError> for EvalError {
    fn from(source: VectorError) -> EvalError {
        EvalError::Vector(source)
    }
}

impl From<serde_json::Error> for EvalError {
    fn from(source: serde_json::Error) -> EvalError {
        EvalError::Serialize(source)
    }
}
