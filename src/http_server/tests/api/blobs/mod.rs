//! HTTP tests for attachment bytes, split by what they exercise: [`serving`] covers the upload and
//! the content-addressed read (the cap, the media type, ranges, misses), and [`delivery`] covers what
//! a message naming a blob does — the request-wide budgets, a missing blob, and the record a
//! delivered attachment leaves in the log.
//!
//! The fixtures and request helpers both halves share live here.

mod delivery;
mod serving;

use crate::http_server::{
    AppState,
    tests::{loopback, test_state},
};
use axum::{body::Body, http::Request};
use std::sync::Arc;
use tower::ServiceExt;
use zuihitsu::{BlobHash, EnvConfig, ManualClock, Server, time::Timestamp};

/// The bytes a test uploads, and their content address.
pub(super) const PNG_BYTES: &[u8] = b"\x89PNG\r\n\x1a\n not really a png, but bytes are bytes";

/// A born agent — the precondition for every delivery test here.
pub(super) fn born_agent() -> Server {
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
pub(super) fn state_with_cap(server: Arc<Server>, cap: usize) -> AppState {
    let mut config = EnvConfig::default();
    config.serving.max_attachment_bytes = cap;
    AppState {
        config: Arc::new(config),
        ..test_state(server)
    }
}

pub(super) fn state_with_message_budgets(
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

pub(super) fn message_body(blob: &BlobHash, names: &[&str]) -> serde_json::Value {
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

pub(super) async fn post_platform_message(
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
pub(super) async fn upload(
    app: axum::Router,
    bytes: &[u8],
    mime: &str,
) -> axum::response::Response {
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
