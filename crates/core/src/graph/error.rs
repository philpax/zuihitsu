use crate::store::StoreError;

#[derive(Debug)]
pub enum GraphError {
    /// The SQLite backend failed.
    Backend(rusqlite::Error),
    /// Reading the log to project from it failed.
    Store(StoreError),
    /// An entry's structured metadata (`told_by` / `visibility`) could not be (de)serialized.
    Serialize(serde_json::Error),
    /// A projected value could not be interpreted — a malformed id or an unknown enum tag — which
    /// means the projection is corrupt (a materializer bug or external tampering), not a typed
    /// failure with a source to delegate to.
    Malformed(String),
}

impl std::fmt::Display for GraphError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GraphError::Backend(error) => write!(f, "materialized graph (backend): {error}"),
            GraphError::Store(error) => write!(f, "materialized graph (store): {error}"),
            GraphError::Serialize(error) => write!(f, "materialized graph (serde): {error}"),
            GraphError::Malformed(message) => {
                write!(f, "materialized graph (malformed): {message}")
            }
        }
    }
}

impl std::error::Error for GraphError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            GraphError::Backend(error) => Some(error),
            GraphError::Store(error) => Some(error),
            GraphError::Serialize(error) => Some(error),
            GraphError::Malformed(_) => None,
        }
    }
}

impl From<rusqlite::Error> for GraphError {
    fn from(error: rusqlite::Error) -> GraphError {
        GraphError::Backend(error)
    }
}

impl From<StoreError> for GraphError {
    fn from(error: StoreError) -> GraphError {
        GraphError::Store(error)
    }
}

impl From<serde_json::Error> for GraphError {
    fn from(error: serde_json::Error) -> GraphError {
        GraphError::Serialize(error)
    }
}
