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
