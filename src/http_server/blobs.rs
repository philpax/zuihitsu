//! Attachment bytes over HTTP: the connector's upload (`POST /platform/blobs`) and the content-
//! addressed read (`GET /blobs/{hash}`).
//!
//! The two sit on opposite sides of the auth boundary, deliberately. The upload lives inside the
//! `/platform` nest, so it takes the platform-key layer and the read-only gate the rest of that
//! surface has. The read is **unauthenticated and top-level**, outside both `/control` and
//! `/platform`: an `<img src>` cannot carry an `Authorization` header, and the 64-hex content hash is
//! itself the capability — it is unguessable, and knowing it means having been told it by someone who
//! had the bytes. Nothing else about the instance is reachable through it.

use axum::{
    Json,
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::Serialize;
use zuihitsu::BlobHash;

use crate::http_server::{AppState, error::ApiError};

/// The response to `POST /platform/blobs`: the content address the bytes are stored at, which the
/// connector then names in the message that carries the file.
#[derive(Serialize)]
pub(super) struct BlobUploaded {
    hash: BlobHash,
}

/// `POST /platform/blobs` — store an attachment's raw bytes, returning their content address. The
/// request's `Content-Type` is the media type they are stored under (`application/octet-stream` when
/// absent), and the body is the bytes themselves rather than any envelope, so a connector streams a
/// download straight through. Idempotent by construction: the same bytes always yield the same
/// address and are stored once.
///
/// A body over `[serving] max_attachment_bytes` is refused whole as a `400`. Truncating would be
/// worse than refusing: the stored address would name bytes nobody sent, and the message referring to
/// it would look perfectly well-formed.
pub(super) async fn upload_blob(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<BlobUploaded>, ApiError> {
    let cap = state.config.serving.max_attachment_bytes;
    if body.len() > cap {
        return Err(ApiError::BadRequest(format!(
            "platform: the attachment is {} bytes, over the {cap}-byte cap \
             ([serving] max_attachment_bytes)",
            body.len()
        )));
    }
    let mime = media_type(&headers);
    let hash = state
        .server
        .put_blob(&body, mime)
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    tracing::info!(%hash, byte_len = body.len(), mime, "stored an attachment");
    Ok(Json(BlobUploaded { hash }))
}

/// `GET /blobs/{hash}` — the bytes stored under a content address, served with the media type they
/// were uploaded under. Unauthenticated: see the module docs for why the hash is the capability.
///
/// The response is immutably cacheable, which is exactly true rather than merely convenient — the
/// address is the content, so what a given URL answers can never change.
///
/// A miss is an explicit `404`, and so is a path segment that is not a well-formed address. Both
/// matter: the router's fallback serves the console's `index.html` for anything unmatched, so
/// answering "not here" is what keeps a missing image from arriving as a page of HTML.
pub(super) async fn blob(
    State(state): State<AppState>,
    Path(hash): Path<String>,
) -> Result<Response, ApiError> {
    let hash: BlobHash = hash
        .parse()
        .map_err(|_| ApiError::NotFound(format!("no blob is stored under {hash}")))?;
    let blob = state
        .server
        .blob(&hash)
        .map_err(|error| ApiError::Internal(error.to_string()))?
        .ok_or_else(|| ApiError::NotFound(format!("no blob is stored under {hash}")))?;
    // A stored media type is whatever an uploading connector sent, so it is rendered through
    // `HeaderValue` rather than trusted: a value carrying anything a header may not hold falls back
    // to the generic type instead of reaching the response.
    let content_type = HeaderValue::from_str(&blob.mime)
        .unwrap_or_else(|_| HeaderValue::from_static(OCTET_STREAM));
    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, content_type),
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static("public, max-age=31536000, immutable"),
            ),
        ],
        blob.bytes,
    )
        .into_response())
}

/// The media type an upload with no usable `Content-Type` is stored under — "some bytes", which is
/// what the request actually said.
const OCTET_STREAM: &str = "application/octet-stream";

/// The longest `Content-Type` accepted. A media type plus its parameters is short; a long one is a
/// connector bug or an attempt to store an essay in the column, and either way the generic type is
/// the honest reading.
const MAX_MEDIA_TYPE_LEN: usize = 255;

/// The media type to store an upload under: the request's `Content-Type` when it is present, ASCII,
/// and of a sane length, and [`OCTET_STREAM`] otherwise.
fn media_type(headers: &HeaderMap) -> &str {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|mime| !mime.is_empty() && mime.len() <= MAX_MEDIA_TYPE_LEN)
        .unwrap_or(OCTET_STREAM)
}
