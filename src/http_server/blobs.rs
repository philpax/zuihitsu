//! Attachment bytes over HTTP: the connector's upload (`POST /platform/blobs`) and the content-
//! addressed read (`GET /blobs/{hash}`).
//!
//! The two sit on opposite sides of the auth boundary, deliberately. The upload lives inside the
//! `/platform` nest, so it takes the platform-key layer and the read-only gate the rest of that
//! surface has. The read is **unauthenticated and top-level**, outside both `/control` and
//! `/platform`: an `<img src>` cannot carry an `Authorization` header, and the 64-hex content hash is
//! itself the capability — it is unguessable, and knowing it means having been told it by someone who
//! had the bytes. Nothing else about the instance is reachable through it.
//!
//! That capability governs *reading* a blob, not what one contains: the bytes are a sender's, and the
//! route is same-origin with the console, so what a response declares itself to be is
//! [`served_media_type`]'s decision rather than the uploader's.

use axum::{
    Json,
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::Serialize;
use zuihitsu::{BlobError, BlobHash, attachment::served_media_type};

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
/// download straight through. Re-uploading the same bytes with the same MIME is idempotent; a
/// different MIME is a `409` because existing attachment metadata is immutable.
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
    let hash = state.server.put_blob(&body, mime).map_err(ApiError::from)?;
    tracing::info!(%hash, byte_len = body.len(), mime, "stored an attachment");
    Ok(Json(BlobUploaded { hash }))
}

/// `GET /blobs/{hash}` — the bytes stored under a content address, served under [`served_media_type`]
/// (the stored type, unless serving it as itself would let a sender's markup run on the console's
/// origin). Unauthenticated: see the module docs for why the hash is the capability.
///
/// A miss is an explicit `404`, and so is a path segment that is not a well-formed address. Both
/// matter: the router's fallback serves the console's `index.html` for anything unmatched, so
/// answering "not here" is what keeps a missing image from arriving as a page of HTML.
///
/// A single-range `Range` is answered as a `206`, read from SQLite without loading the rest — how a
/// viewer excerpts the head of a long text file. Every response advertises `Accept-Ranges: bytes`.
pub(super) async fn blob(
    State(state): State<AppState>,
    Path(hash): Path<String>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let hash: BlobHash = hash
        .parse()
        .map_err(|_| ApiError::NotFound(format!("no blob is stored under {hash}")))?;
    let missing = || ApiError::NotFound(format!("no blob is stored under {hash}"));
    let internal = |error: BlobError| ApiError::Internal(error.to_string());

    let Some(requested) = requested_range(&headers) else {
        let blob = state
            .server
            .blob(&hash)
            .map_err(internal)?
            .ok_or_else(missing)?;
        return Ok((StatusCode::OK, common_headers(&blob.mime), blob.bytes).into_response());
    };

    let meta = state
        .server
        .blob_meta(&hash)
        .map_err(internal)?
        .ok_or_else(missing)?;
    // A suffix range and an open-ended one both resolve against the stored length, so read it first.
    let Some((start, end)) = requested.resolve(meta.byte_len) else {
        // Unsatisfiable (RFC 9110 §15.5.17): state the size, so a retry needs no extra round trip.
        return Ok((
            StatusCode::RANGE_NOT_SATISFIABLE,
            [
                (
                    header::CONTENT_RANGE,
                    HeaderValue::from_str(&format!("bytes */{}", meta.byte_len))
                        .expect("a byte count renders as a header value"),
                ),
                (header::ACCEPT_RANGES, HeaderValue::from_static("bytes")),
            ],
        )
            .into_response());
    };
    let range = state
        .server
        .blob_range(&hash, start, end - start + 1)
        .map_err(internal)?
        .ok_or_else(missing)?;
    Ok((
        StatusCode::PARTIAL_CONTENT,
        common_headers(&range.mime),
        [(
            header::CONTENT_RANGE,
            HeaderValue::from_str(&format!("bytes {start}-{end}/{}", range.byte_len))
                .expect("a byte range renders as a header value"),
        )],
        range.bytes,
    )
        .into_response())
}

/// The headers every blob response carries, whole or partial.
///
/// The media type is [`served_media_type`]'s, rendered through `HeaderValue` rather than trusted: a
/// stored value a header may not hold falls back to the generic type. `nosniff` is what makes that
/// type binding, since a `text/plain` body opening with `<html>` is otherwise a sniffing candidate.
/// The immutable caching is exactly true: the address is the content.
fn common_headers(mime: &str) -> [(header::HeaderName, HeaderValue); 4] {
    [
        (
            header::CONTENT_TYPE,
            HeaderValue::from_str(served_media_type(mime))
                .unwrap_or_else(|_| HeaderValue::from_static(OCTET_STREAM)),
        ),
        (
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=31536000, immutable"),
        ),
        (header::ACCEPT_RANGES, HeaderValue::from_static("bytes")),
        (
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ),
    ]
}

/// One byte range as the request asked for it, before the stored length is known.
#[derive(Debug, PartialEq, Eq)]
enum RequestedRange {
    /// `bytes=start-end` or `bytes=start-`, the end resolved against the stored length.
    From { start: u64, end: Option<u64> },
    /// `bytes=-suffix`: the last `suffix` bytes, however long the blob turns out to be.
    Suffix(u64),
}

impl RequestedRange {
    /// The inclusive `(start, end)` this range addresses in a blob of `byte_len` bytes, or `None`
    /// when it addresses nothing there — a start at or past the end, or a zero-length suffix. An end
    /// past the last byte is clamped rather than refused, which is what a client asking for "the
    /// first 4 KiB" of a shorter file means.
    fn resolve(&self, byte_len: u64) -> Option<(u64, u64)> {
        if byte_len == 0 {
            return None;
        }
        let last = byte_len - 1;
        match *self {
            RequestedRange::From { start, end } => {
                (start <= last).then(|| (start, end.unwrap_or(last).min(last)))
            }
            RequestedRange::Suffix(suffix) => {
                (suffix > 0).then(|| (byte_len.saturating_sub(suffix), last))
            }
        }
    }
}

/// The single byte range a request asked for, or `None` when it asked for none.
///
/// Anything unrecognised — another unit, several ranges, a malformed spec — reads as `None` and is
/// served whole, which RFC 9110 §14.2 permits and which beats erroring at the reader.
fn requested_range(headers: &HeaderMap) -> Option<RequestedRange> {
    let spec = headers
        .get(header::RANGE)?
        .to_str()
        .ok()?
        .trim()
        .strip_prefix("bytes=")?
        .trim();
    if spec.contains(',') {
        return None;
    }
    let (start, end) = spec.split_once('-')?;
    let (start, end) = (start.trim(), end.trim());
    if start.is_empty() {
        return Some(RequestedRange::Suffix(end.parse().ok()?));
    }
    let start = start.parse().ok()?;
    let end = if end.is_empty() {
        None
    } else {
        let end: u64 = end.parse().ok()?;
        if end < start {
            return None;
        }
        Some(end)
    };
    Some(RequestedRange::From { start, end })
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
