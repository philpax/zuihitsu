//! Storing an attachment's bytes and reading them back: the cap, the media type a response declares, byte ranges, and what a miss answers.

use super::{PNG_BYTES, state_with_cap, upload};
use crate::http_server::{router, tests::test_state};
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use std::sync::Arc;
use tower::ServiceExt;
use zuihitsu::{BlobHash, ManualClock, Server, time::Timestamp};

#[tokio::test]
async fn an_uploaded_blob_is_fetched_back_by_its_address() {
    let server =
        Arc::new(Server::in_memory(Box::new(ManualClock::new(Timestamp::from_millis(0)))).unwrap());
    let app = router(test_state(server));

    let response = upload(app.clone(), PNG_BYTES, "image/png").await;
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let hash = body["hash"].as_str().expect("the response names the hash");
    assert_eq!(hash, BlobHash::of(PNG_BYTES).as_str());

    // The read is top-level and needs no key: the hash is the capability.
    let fetched = app
        .oneshot(
            Request::builder()
                .uri(format!("/blobs/{hash}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(fetched.status(), StatusCode::OK);
    assert_eq!(fetched.headers()["content-type"], "image/png");
    assert_eq!(
        fetched.headers()["cache-control"],
        "public, max-age=31536000, immutable"
    );
    let fetched_bytes = axum::body::to_bytes(fetched.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(fetched_bytes.as_ref(), PNG_BYTES);
}

#[tokio::test]
async fn a_text_attachment_is_served_as_plain_text_so_markup_renders_rather_than_running() {
    // The bytes are a sender's, and this route is same-origin with the console: serving an uploaded
    // `text/html` as itself would run its script there. Plain text still opens in place — the reader
    // sees the file, and there is nothing for the browser to execute.
    let server =
        Arc::new(Server::in_memory(Box::new(ManualClock::new(Timestamp::from_millis(0)))).unwrap());
    let app = router(test_state(server.clone()));

    let markup = b"<html><script>alert(document.domain)</script></html>";
    for (bytes, stored, served) in [
        (&markup[..], "text/html", "text/plain; charset=utf-8"),
        (b"<svg/>", "image/svg+xml", "text/plain; charset=utf-8"),
        (
            b"{\"a\":1}",
            "application/json",
            "text/plain; charset=utf-8",
        ),
        // An image the model can perceive is inert and stays itself, or the console would have
        // nothing to put in an `<img>`. So does a type that is neither, which `nosniff` keeps from
        // being sniffed into markup.
        (b"\x89PNG fake", "image/png", "image/png"),
        (b"%PDF-1.7 fake", "application/pdf", "application/pdf"),
    ] {
        upload(app.clone(), bytes, stored).await;
        let (status, headers, body) = fetch_range(app.clone(), &BlobHash::of(bytes), None).await;
        assert_eq!(status, StatusCode::OK, "serving {stored}");
        assert_eq!(headers["content-type"], served, "serving {stored}");
        assert_eq!(headers["x-content-type-options"], "nosniff");
        // Only the declared type changes; the bytes are served exactly as stored.
        assert_eq!(body, bytes, "serving {stored}");
    }

    // The stored record is untouched — the downgrade is a decision about one response.
    let stored = server.blob_meta(&BlobHash::of(markup)).unwrap().unwrap();
    assert_eq!(stored.mime, "text/html");
}

/// Fetch `path` with an optional `Range` header, returning the status, the headers, and the body —
/// the three a ranged read is judged on.
async fn fetch_range(
    app: axum::Router,
    hash: &BlobHash,
    range: Option<&str>,
) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let mut request = Request::builder().uri(format!("/blobs/{hash}"));
    if let Some(range) = range {
        request = request.header("range", range);
    }
    let response = app
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, headers, body.to_vec())
}

#[tokio::test]
async fn a_ranged_read_answers_the_window_the_reader_asked_for() {
    // The console excerpts the head of a long text attachment this way, so it renders the opening of
    // a large file without pulling the whole thing down.
    let server =
        Arc::new(Server::in_memory(Box::new(ManualClock::new(Timestamp::from_millis(0)))).unwrap());
    let app = router(test_state(server));
    let text = b"0123456789abcdef";
    upload(app.clone(), text, "text/plain").await;
    let hash = BlobHash::of(text);

    // A bounded window, an open-ended one, a suffix, and an end past the last byte (clamped, which is
    // what "the first 4 KiB" of a shorter file means).
    for (spec, expected, content_range) in [
        ("bytes=0-3", &b"0123"[..], "bytes 0-3/16"),
        ("bytes=4-", &b"456789abcdef"[..], "bytes 4-15/16"),
        ("bytes=-4", &b"cdef"[..], "bytes 12-15/16"),
        ("bytes=10-999", &b"abcdef"[..], "bytes 10-15/16"),
    ] {
        let (status, headers, body) = fetch_range(app.clone(), &hash, Some(spec)).await;
        assert_eq!(status, StatusCode::PARTIAL_CONTENT, "asking for {spec}");
        assert_eq!(headers["content-range"], content_range, "asking for {spec}");
        assert_eq!(headers["content-type"], "text/plain; charset=utf-8");
        assert_eq!(headers["accept-ranges"], "bytes");
        assert_eq!(body, expected, "asking for {spec}");
    }

    // No range at all is the whole blob, still advertising that ranges are available.
    let (status, headers, body) = fetch_range(app.clone(), &hash, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["accept-ranges"], "bytes");
    assert_eq!(body, text);
}

#[tokio::test]
async fn an_unsatisfiable_range_is_a_416_naming_the_size_and_an_unparsed_one_serves_the_whole_blob()
{
    let server =
        Arc::new(Server::in_memory(Box::new(ManualClock::new(Timestamp::from_millis(0)))).unwrap());
    let app = router(test_state(server));
    let text = b"0123456789abcdef";
    upload(app.clone(), text, "text/plain").await;
    let hash = BlobHash::of(text);

    // Past the end: the response states the size the client should have asked within.
    let (status, headers, _) = fetch_range(app.clone(), &hash, Some("bytes=16-20")).await;
    assert_eq!(status, StatusCode::RANGE_NOT_SATISFIABLE);
    assert_eq!(headers["content-range"], "bytes */16");

    // A unit we do not speak, several ranges, a backwards range, and a malformed spec are all served
    // whole — RFC 9110 §14.2 lets a server ignore a Range it does not understand, and a reader is
    // better served the file than an error.
    for spec in ["items=0-3", "bytes=0-3,8-9", "bytes=9-2", "bytes=abc"] {
        let (status, _, body) = fetch_range(app.clone(), &hash, Some(spec)).await;
        assert_eq!(status, StatusCode::OK, "asking for {spec}");
        assert_eq!(body, text, "asking for {spec}");
    }
}

#[tokio::test]
async fn an_unknown_or_malformed_address_is_a_404() {
    // Both must be an explicit 404: the router's fallback serves the console's `index.html` for
    // anything unmatched, so a miss that fell through would arrive as a page of HTML.
    let server =
        Arc::new(Server::in_memory(Box::new(ManualClock::new(Timestamp::from_millis(0)))).unwrap());
    let app = router(test_state(server));

    for path in [
        format!("/blobs/{}", BlobHash::of(b"never uploaded")),
        "/blobs/not-a-hash".to_owned(),
        // A well-formed-looking address in the wrong case, and one of the wrong length.
        format!("/blobs/{}", BlobHash::of(b"x").to_string().to_uppercase()),
    ] {
        let response = app
            .clone()
            .oneshot(Request::builder().uri(&path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "fetching {path}");
    }
}

#[tokio::test]
async fn an_over_cap_upload_is_refused_rather_than_truncated() {
    let server =
        Arc::new(Server::in_memory(Box::new(ManualClock::new(Timestamp::from_millis(0)))).unwrap());
    let app = router(state_with_cap(server.clone(), 8));

    let response = upload(app.clone(), b"nine byte", "text/plain").await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    // Nothing was stored: a rejected upload leaves no half-blob behind.
    assert_eq!(server.blob(&BlobHash::of(b"nine byte")).unwrap(), None);

    // A body at the cap is fine — the refusal is over it, not at it.
    let response = upload(app, b"eight by", "text/plain").await;
    assert_eq!(response.status(), StatusCode::OK);
}
