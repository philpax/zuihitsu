//! What a message naming a blob does: the request-wide budgets, a blob the store never held, and the record a delivered attachment leaves.

use super::{
    PNG_BYTES, born_agent, message_body, post_platform_message, state_with_message_budgets, upload,
};
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
    Attachment, AttachmentKind, BlobHash, Completion, EventPayload, MemoryStore, ScriptedModel,
    Seq, buffer_turns,
};

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
    // The stored type is preserved (asserted above); the response states the charset the text is
    // served under — see the plain-text serving test.
    assert_eq!(read.headers()["content-type"], "text/plain; charset=utf-8");
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
