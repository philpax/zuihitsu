//! The request error rendered as an HTTP response, shared by both surfaces' handlers. Distinct from
//! the startup [`super::ServeError`]: this is a per-request failure, that is a boot failure.

use axum::{Json, http::StatusCode, response::IntoResponse};
use serde::Serialize;
use zuihitsu::ServerError;

/// An error rendered as an HTTP response. A [`ServerError`] is an infrastructure/processing failure →
/// `500`; a `NotFound` is a named resource that does not exist → `404`. Malformed request bodies are
/// rejected at the axum extractor (`400`) before a handler runs, so that case never reaches here.
pub(super) enum ApiError {
    Server(ServerError),
    NotFound(String),
    /// A conversing endpoint was called but no model is configured.
    NoModel,
    /// The snapshot endpoint was called but snapshotting is disabled (`[snapshots] enabled = false`).
    SnapshotsDisabled,
}

impl From<ServerError> for ApiError {
    fn from(error: ServerError) -> Self {
        ApiError::Server(error)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        let (status, message) = match self {
            ApiError::Server(error) => {
                tracing::error!(%error, "request failed");
                (StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
            }
            ApiError::NotFound(message) => (StatusCode::NOT_FOUND, message),
            ApiError::NoModel => (
                StatusCode::SERVICE_UNAVAILABLE,
                "no model endpoint is configured".to_owned(),
            ),
            ApiError::SnapshotsDisabled => (
                StatusCode::CONFLICT,
                "snapshots are disabled ([snapshots] enabled = false)".to_owned(),
            ),
        };
        (status, Json(ErrorBody { error: message })).into_response()
    }
}

#[derive(Serialize)]
struct ErrorBody {
    error: String,
}
