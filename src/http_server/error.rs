//! The request error rendered as an HTTP response, shared by both surfaces' handlers. Distinct from
//! the startup [`crate::http_server::ServeError`]: this is a per-request failure, that is a boot failure.

use axum::{Json, http::StatusCode, response::IntoResponse};
use serde::Serialize;
use zuihitsu::{BlobError, ServerError};

/// An error rendered as an HTTP response. A [`ServerError`] is an infrastructure/processing failure →
/// `500`; a `NotFound` is a named resource that does not exist → `404`. Malformed request bodies are
/// rejected at the axum extractor (`400`) before a handler runs, so that case never reaches here.
pub(super) enum ApiError {
    Server(ServerError),
    NotFound(String),
    /// A request the handler rejected on its own terms — malformed or out-of-range operator input the
    /// axum extractor could not catch (e.g. an empty or over-long `self` edit).
    BadRequest(String),
    /// The request conflicts with immutable state already recorded by the server, such as a blob's
    /// existing media type.
    Conflict(String),
    /// A conversing endpoint was called but no model is configured.
    NoModel,
    /// The snapshot endpoint was called but snapshotting is disabled (`[snapshots] enabled = false`).
    SnapshotsDisabled,
    /// The server is booted in read-only mode, which refuses every mutating endpoint. Restart
    /// without `--read-only` to converse, mutate, or act.
    ReadOnly,
    /// The metrics endpoint was called but the recorder could not be installed at boot.
    MetricsDisabled,
    /// A detached task backing the request failed to join — a panic in the turn task, surfaced as
    /// an internal failure rather than swallowed.
    Internal(String),
}

impl From<ServerError> for ApiError {
    fn from(error: ServerError) -> Self {
        ApiError::Server(error)
    }
}

impl From<BlobError> for ApiError {
    fn from(error: BlobError) -> Self {
        match error {
            BlobError::MimeConflict {
                hash,
                existing_mime,
                requested_mime,
            } => ApiError::Conflict(format!(
                "platform: blob {hash} already uses MIME {existing_mime}, so it cannot be uploaded as {requested_mime}"
            )),
            other => ApiError::Internal(other.to_string()),
        }
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
            ApiError::BadRequest(message) => (StatusCode::BAD_REQUEST, message),
            ApiError::Conflict(message) => (StatusCode::CONFLICT, message),
            ApiError::NoModel => (
                StatusCode::SERVICE_UNAVAILABLE,
                "no model endpoint is configured".to_owned(),
            ),
            ApiError::SnapshotsDisabled => (
                StatusCode::CONFLICT,
                "snapshots are disabled ([snapshots] enabled = false)".to_owned(),
            ),
            ApiError::ReadOnly => (
                StatusCode::CONFLICT,
                "the server is booted in read-only mode; restart without --read-only to converse, \
                 mutate, or act"
                    .to_owned(),
            ),
            ApiError::MetricsDisabled => (
                StatusCode::SERVICE_UNAVAILABLE,
                "the metrics recorder is not installed".to_owned(),
            ),
            ApiError::Internal(message) => {
                tracing::error!(%message, "request failed");
                (StatusCode::INTERNAL_SERVER_ERROR, message)
            }
        };
        (status, Json(ErrorBody { error: message })).into_response()
    }
}

#[derive(Serialize)]
struct ErrorBody {
    error: String,
}
