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
//! That capability governs *reading* a blob, not what a blob contains: the bytes are whatever a
//! sender uploaded, and the route is same-origin with the console. What a response may therefore
//! declare itself to be is decided by [`served_media_type`], not by the uploader.

use axum::{
    Json,
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::Serialize;
use zuihitsu::{AttachmentKind, BlobError, BlobHash};

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
/// A `Range` header asking for one byte range is answered with that window as a `206`, read from
/// SQLite without loading the rest — how the console excerpts the head of a text attachment without
/// pulling a 16 MiB log down to show four thousand characters of it. Every response advertises
/// `Accept-Ranges: bytes`, so a client knows the option is there.
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
    // The stored length resolves a suffix range and bounds an open-ended one, so it is read before the
    // bytes: a range is only meaningful against the size of what it addresses.
    let Some((start, end)) = requested.resolve(meta.byte_len) else {
        // Unsatisfiable, per RFC 9110 §15.5.17: the response states the size the client should have
        // asked within, so a retry needs no extra round trip.
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
/// The media type is [`served_media_type`]'s rather than the stored one, and it is rendered through
/// `HeaderValue` rather than trusted: a stored value carrying anything a header may not hold falls
/// back to the generic type instead of reaching the response.
///
/// `nosniff` is what makes that served type binding. Without it a browser may sniff a `text/plain`
/// body that opens with `<html>` back into markup, which would undo the downgrade the served type
/// exists to perform.
///
/// The response is immutably cacheable, which is exactly true rather than merely convenient — the
/// address is the content, so what a given URL answers can never change.
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

/// The media type a blob is *served* under, which is not always the one it was stored under.
///
/// The bytes are uploader-controlled and this route is same-origin with the console, so a stored
/// `text/html` served as itself would run script on the console's origin — the address being
/// unguessable stops a stranger reading a blob, not a sender choosing what their own blob contains.
/// Anything the system already treats as text is therefore served as `text/plain`, which a browser
/// renders inline as text: the reader still opens the file in place and sees exactly what it says,
/// and there is no markup for the browser to execute. It is the closest thing the web has to a
/// view-source media type — HTML has no inline-as-source rendering the way XML has its tree viewer,
/// so the choice is between rendering it as a document, forcing a download, and this.
///
/// An image type is served verbatim: the four [`AttachmentKind::Image`] types are inert raster
/// formats, and downgrading them would leave the console with nothing to put in an `<img>`. Anything
/// else keeps its stored type too — the executable-in-a-browser types (`text/html`,
/// `application/xhtml+xml`, `image/svg+xml`) all classify as text and are covered above, and
/// `nosniff` stops the rest from being sniffed into one — so a PDF still opens in the viewer.
///
/// The stored metadata is untouched: this is a decision about one response, and the attachment record
/// the log holds still says what was uploaded.
fn served_media_type(mime: &str) -> &str {
    match AttachmentKind::of_mime(mime) {
        AttachmentKind::Text => PLAIN_TEXT,
        AttachmentKind::Image | AttachmentKind::Opaque => mime,
    }
}

/// The media type every text-classified attachment is served under. The charset is stated because a
/// browser left to guess one may pick a legacy encoding, and the agent's own inlining already reads
/// these bytes as UTF-8.
const PLAIN_TEXT: &str = "text/plain; charset=utf-8";

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
/// Anything this does not understand — a unit other than `bytes`, several ranges, a malformed spec —
/// reads as `None` and is answered with the whole blob, which RFC 9110 §14.2 permits ("a server MAY
/// ignore the Range header field"). A partial reader that is not understood is better served the
/// whole file than an error.
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
