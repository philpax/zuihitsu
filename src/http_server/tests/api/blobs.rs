//! HTTP tests for attachment bytes: the connector's upload, the unauthenticated content-addressed
//! read, the byte cap, and what a message naming a blob does — both when the bytes are there and when
//! they are not. The recording end is covered too: a delivered attachment must reach the event log
//! and survive a replay through the buffer.

use crate::http_server::{
    AppState, router,
    tests::{loopback, test_state},
};
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use std::sync::Arc;
use tower::ServiceExt;
use zuihitsu::{
    Attachment, AttachmentKind, BlobHash, Completion, EnvConfig, EventPayload, ManualClock,
    MemoryStore, ScriptedModel, Seq, Server, buffer_turns, time::Timestamp,
};

/// The bytes a test uploads, and their content address.
const PNG_BYTES: &[u8] = b"\x89PNG\r\n\x1a\n not really a png, but bytes are bytes";

/// A born agent — the precondition for every delivery test here.
fn born_agent() -> Server {
    let server = Server::in_memory(Box::new(ManualClock::new(Timestamp::from_millis(0)))).unwrap();
    server
        .control()
        .create_agent(&zuihitsu::SeedSelf {
            agent_name: "Kestrel".to_owned(),
            persona: "An assistant.".to_owned(),
            seed_entries: vec![],
        })
        .unwrap();
    server
}

/// An app state whose attachment cap is `cap` bytes, for the over-cap refusal.
fn state_with_cap(server: Arc<Server>, cap: usize) -> AppState {
    let mut config = EnvConfig::default();
    config.serving.max_attachment_bytes = cap;
    AppState {
        config: Arc::new(config),
        ..test_state(server)
    }
}

fn state_with_message_budgets(
    server: Arc<Server>,
    count: usize,
    bytes: u64,
    model: Option<Arc<dyn zuihitsu::ModelClient>>,
) -> AppState {
    let mut config = EnvConfig::default();
    config.serving.max_message_attachment_count = count;
    config.serving.max_message_attachment_bytes = bytes;
    AppState {
        config: Arc::new(config),
        model,
        ..test_state(server)
    }
}

fn message_body(blob: &BlobHash, names: &[&str]) -> serde_json::Value {
    serde_json::json!({
        "scope_path": "general",
        "messages": [{
            "sender": "rowan",
            "text": "look at these",
            "attachments": names.iter().map(|name| serde_json::json!({
                "name": name,
                "blob": blob.as_str(),
            })).collect::<Vec<_>>(),
        }],
        "present": ["rowan"],
    })
}

async fn post_platform_message(
    app: axum::Router,
    body: serde_json::Value,
) -> axum::response::Response {
    app.oneshot(
        Request::builder()
            .extension(loopback())
            .method("POST")
            .uri("/platform/messages")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
    )
    .await
    .unwrap()
}

/// Upload `bytes` as `mime` and return the response.
async fn upload(app: axum::Router, bytes: &[u8], mime: &str) -> axum::response::Response {
    app.oneshot(
        Request::builder()
            .extension(loopback())
            .method("POST")
            .uri("/platform/blobs")
            .header("content-type", mime)
            .body(Body::from(bytes.to_vec()))
            .unwrap(),
    )
    .await
    .unwrap()
}

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
        assert_eq!(headers["content-type"], "text/plain");
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

#[tokio::test]
async fn platform_messages_accepts_exact_attachment_count_limit_and_rejects_one_over() {
    let server = Arc::new(born_agent());
    let model: Arc<dyn zuihitsu::ModelClient> = Arc::new(ScriptedModel::new([
        Completion::Reply("Noted.".to_owned()),
        Completion::Reply("Noted again.".to_owned()),
    ]));
    let hash = server.put_blob(b"one", "text/plain").unwrap();
    let app = router(state_with_message_budgets(
        server.clone(),
        2,
        6,
        Some(model),
    ));

    let exact = post_platform_message(app.clone(), message_body(&hash, &["a", "b"])).await;
    assert_eq!(exact.status(), StatusCode::OK);
    let over = post_platform_message(app, message_body(&hash, &["a", "b", "c"])).await;
    assert_eq!(over.status(), StatusCode::BAD_REQUEST);
    let body = axum::body::to_bytes(over.into_body(), usize::MAX)
        .await
        .unwrap();
    assert!(String::from_utf8_lossy(&body).contains("max_message_attachment_count"));
}

#[tokio::test]
async fn platform_messages_accepts_exact_attachment_byte_limit_and_rejects_one_over() {
    let server = Arc::new(born_agent());
    let model: Arc<dyn zuihitsu::ModelClient> = Arc::new(ScriptedModel::new([
        Completion::Reply("Noted.".to_owned()),
        Completion::Reply("Noted again.".to_owned()),
    ]));
    let hash = server.put_blob(b"123", "text/plain").unwrap();
    let app = router(state_with_message_budgets(
        server.clone(),
        4,
        6,
        Some(model),
    ));

    let exact = post_platform_message(app.clone(), message_body(&hash, &["a", "b"])).await;
    assert_eq!(exact.status(), StatusCode::OK);
    let over = post_platform_message(app, message_body(&hash, &["a", "b", "c"])).await;
    assert_eq!(over.status(), StatusCode::BAD_REQUEST);
    let body = axum::body::to_bytes(over.into_body(), usize::MAX)
        .await
        .unwrap();
    assert!(String::from_utf8_lossy(&body).contains("max_message_attachment_bytes"));
}

#[tokio::test]
async fn platform_message_counts_repeated_blob_references_individually() {
    let server = Arc::new(born_agent());
    let model: Arc<dyn zuihitsu::ModelClient> =
        Arc::new(ScriptedModel::new([Completion::Reply("Noted.".to_owned())]));
    let hash = server.put_blob(b"123", "text/plain").unwrap();
    let app = router(state_with_message_budgets(server, 3, 100, Some(model)));
    let response = post_platform_message(app, message_body(&hash, &["a", "a", "a", "a"])).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert!(String::from_utf8_lossy(&body).contains("max_message_attachment_count"));
}

#[tokio::test]
async fn platform_messages_rejects_missing_blob_before_no_model() {
    let server = Arc::new(born_agent());
    let before = server.control().events().unwrap().len();
    let app = router(state_with_message_budgets(
        server.clone(),
        32,
        64 * 1024 * 1024,
        None,
    ));
    let missing = BlobHash::of(b"never uploaded");
    let response = post_platform_message(app, message_body(&missing, &["missing.txt"])).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let text = String::from_utf8_lossy(&body);
    assert!(text.contains(missing.as_str()));
    assert!(!text.contains("no model"));
    assert_eq!(server.control().events().unwrap().len(), before);
}

#[tokio::test]
async fn platform_messages_rejects_over_budget_before_turn() {
    let server = Arc::new(born_agent());
    let before = server.control().events().unwrap().len();
    let hash = server.put_blob(b"123", "text/plain").unwrap();
    let app = router(state_with_message_budgets(server.clone(), 0, 0, None));
    let response = post_platform_message(app, message_body(&hash, &["a"])).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert!(String::from_utf8_lossy(&body).contains("max_message_attachment_count"));
    assert_eq!(server.control().events().unwrap().len(), before);
}

#[tokio::test]
async fn uploading_same_bytes_with_a_different_mime_returns_conflict_and_preserves_metadata() {
    let server = Arc::new(born_agent());
    let app = router(test_state(server.clone()));
    let first = upload(app.clone(), b"same bytes", "text/plain").await;
    assert_eq!(first.status(), StatusCode::OK);
    let second = upload(app.clone(), b"same bytes", "image/png").await;
    assert_eq!(second.status(), StatusCode::CONFLICT);
    let bytes = axum::body::to_bytes(second.into_body(), usize::MAX)
        .await
        .unwrap();
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains(BlobHash::of(b"same bytes").as_str()));
    assert!(text.contains("text/plain") && text.contains("image/png"));
    assert_eq!(
        server
            .blob_meta(&BlobHash::of(b"same bytes"))
            .unwrap()
            .unwrap()
            .mime,
        "text/plain"
    );
    let read = app
        .oneshot(
            Request::builder()
                .uri(format!("/blobs/{}", BlobHash::of(b"same bytes")))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(read.headers()["content-type"], "text/plain");
}

#[tokio::test]
async fn a_message_naming_an_unknown_blob_is_refused() {
    // The bytes must be uploaded before the message that carries them; otherwise the turn would
    // record an attachment nobody can read.
    let model: Arc<dyn zuihitsu::ModelClient> =
        Arc::new(ScriptedModel::new([Completion::Reply("Hi.".to_owned())]));
    let app = router(AppState {
        model: Some(model),
        ..test_state(Arc::new(born_agent()))
    });

    let body = serde_json::json!({
        "scope_path": "general",
        "messages": [{
            "sender": "rowan",
            "text": "look at this",
            "attachments": [{
                "name": "plan.png",
                "mime": "image/png",
                "blob": BlobHash::of(PNG_BYTES).as_str(),
                "byte_len": PNG_BYTES.len(),
                "kind": "Image",
            }],
        }],
        "present": ["rowan"],
    });
    let response = app
        .oneshot(
            Request::builder()
                .extension(loopback())
                .method("POST")
                .uri("/platform/messages")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let error: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(
        error["error"]
            .as_str()
            .unwrap()
            .contains(BlobHash::of(PNG_BYTES).as_str()),
        "the refusal names the missing hash: {error}"
    );
}

#[tokio::test]
async fn a_delivered_attachment_is_recorded_and_replays_into_the_buffer() {
    let server = Arc::new(born_agent());
    let model: Arc<dyn zuihitsu::ModelClient> =
        Arc::new(ScriptedModel::new([Completion::Reply("Noted.".to_owned())]));
    let app = router(AppState {
        model: Some(model),
        ..test_state(server.clone())
    });

    let uploaded = upload(app.clone(), PNG_BYTES, "image/png").await;
    assert_eq!(uploaded.status(), StatusCode::OK);

    let body = serde_json::json!({
        "scope_path": "general",
        "messages": [{
            "sender": "rowan",
            "text": "look at this",
            "attachments": [{
                "name": "plan.png",
                // The body's media type, length, and kind are ignored: the stored blob is
                // authoritative, so a connector cannot describe a blob as something it is not.
                "mime": "application/pdf",
                "blob": BlobHash::of(PNG_BYTES).as_str(),
                "byte_len": 1,
                "kind": "Opaque",
            }],
        }],
        "present": ["rowan"],
    });
    let response = app
        .oneshot(
            Request::builder()
                .extension(loopback())
                .method("POST")
                .uri("/platform/messages")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let expected = Attachment {
        name: "plan.png".to_owned(),
        mime: "image/png".into(),
        blob: BlobHash::of(PNG_BYTES),
        byte_len: PNG_BYTES.len() as u64,
        kind: AttachmentKind::Image,
    };

    // The participant turn recorded the attachment as the blob store describes it.
    let events = server.control().events().unwrap();
    let (conversation, recorded) = events
        .iter()
        .find_map(|event| match &event.payload {
            EventPayload::ConversationTurn {
                conversation,
                attachments,
                ..
            } if !attachments.is_empty() => Some((*conversation, attachments.clone())),
            _ => None,
        })
        .expect("a turn carries the attachment");
    assert_eq!(recorded, vec![expected.clone()]);

    // And a replay of that log through the buffer keeps it, so a later turn sees the same files the
    // live one did.
    let replayed = MemoryStore::from_events(events);
    let turns = buffer_turns(&replayed, conversation, Seq::ZERO).unwrap();
    let attachments: Vec<Attachment> = turns
        .into_iter()
        .flat_map(|turn| turn.attachments)
        .collect();
    assert_eq!(attachments, vec![expected]);
}
