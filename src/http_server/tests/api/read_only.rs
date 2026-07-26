//! Read-only mode: every mutating handler returns `409` when the server is booted read-only, and
//! read handlers return `200`. The gate is centralised in `refuse_if_read_only`, so representative
//! coverage exercises the representative mutating endpoints across both surfaces.

use crate::http_server::{
    router,
    tests::{loopback, test_state_read_only},
};
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use std::sync::Arc;
use tower::ServiceExt;
use zuihitsu::{ManualClock, Server, time::Timestamp};

/// A born agent wrapped in a read-only `AppState`.
fn read_only_app() -> axum::Router {
    let server =
        Arc::new(Server::in_memory(Box::new(ManualClock::new(Timestamp::from_millis(0)))).unwrap());
    server
        .control()
        .create_agent(&zuihitsu::SeedSelf {
            agent_name: "Kestrel".to_owned(),
            persona: "An assistant.".to_owned(),
            seed_entries: vec![],
        })
        .unwrap();
    router(test_state_read_only(server))
}

async fn assert_conflict(app: axum::Router, request: Request<Body>) {
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
}

async fn assert_ok(app: axum::Router, request: Request<Body>) {
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

// --- Control surface: mutating handlers return 409 ---

#[tokio::test]
async fn read_only_refuses_create_agent() {
    let body = serde_json::json!({
        "agent_name": "Kestrel",
        "persona": "An assistant.",
        "seed_entries": [],
    });
    assert_conflict(
        read_only_app(),
        Request::builder()
            .extension(loopback())
            .method("POST")
            .uri("/control/agent")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
    )
    .await;
}

#[tokio::test]
async fn read_only_refuses_set_settings() {
    let body = serde_json::json!({});
    assert_conflict(
        read_only_app(),
        Request::builder()
            .extension(loopback())
            .method("PUT")
            .uri("/control/settings")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
    )
    .await;
}

#[tokio::test]
async fn read_only_refuses_retract_entry() {
    let body = serde_json::json!({
        "memory": "self",
        "entry": "01J00000000000000000000000",
        "reason": "test",
    });
    assert_conflict(
        read_only_app(),
        Request::builder()
            .extension(loopback())
            .method("POST")
            .uri("/control/retract")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
    )
    .await;
}

#[tokio::test]
async fn read_only_refuses_run_lua() {
    let body = serde_json::json!({"script": "return 1", "allow_mcp": false, "allow_web": false});
    assert_conflict(
        read_only_app(),
        Request::builder()
            .extension(loopback())
            .method("POST")
            .uri("/control/lua")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
    )
    .await;
}

#[tokio::test]
async fn read_only_refuses_maintenance_consolidate() {
    assert_conflict(
        read_only_app(),
        Request::builder()
            .extension(loopback())
            .method("POST")
            .uri("/control/maintenance/consolidate")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
}

#[tokio::test]
async fn read_only_refuses_snapshot() {
    // The snapshot handler has its own 409 path (`SnapshotsDisabled`), but read-only mode must
    // gate first — both are 409, so this test also checks the body to distinguish them.
    let app = read_only_app();
    let response = app
        .oneshot(
            Request::builder()
                .extension(loopback())
                .method("POST")
                .uri("/control/snapshot")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(
        body["error"]
            .as_str()
            .is_some_and(|msg| msg.contains("read-only mode")),
        "the 409 must come from the read-only gate, not SnapshotsDisabled"
    );
}

// --- Control surface: read handlers return 200 ---

#[tokio::test]
async fn read_only_serves_genesis() {
    assert_ok(
        read_only_app(),
        Request::builder()
            .extension(loopback())
            .uri("/control/genesis")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
}

#[tokio::test]
async fn read_only_serves_events() {
    assert_ok(
        read_only_app(),
        Request::builder()
            .extension(loopback())
            .uri("/control/events")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
}

#[tokio::test]
async fn read_only_serves_settings() {
    assert_ok(
        read_only_app(),
        Request::builder()
            .extension(loopback())
            .uri("/control/settings")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
}

#[tokio::test]
async fn read_only_serves_lua_api() {
    assert_ok(
        read_only_app(),
        Request::builder()
            .extension(loopback())
            .uri("/control/lua-api")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
}

// --- Platform surface: mutating handlers return 409 ---

#[tokio::test]
async fn read_only_refuses_platform_message() {
    let body = serde_json::json!({
        "scope_path": "general",
        "messages": [{ "sender": "dave", "text": "hello" }],
        "present": ["dave"],
    });
    assert_conflict(
        read_only_app(),
        Request::builder()
            .extension(loopback())
            .method("POST")
            .uri("/platform/messages")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
    )
    .await;
}

#[tokio::test]
async fn read_only_refuses_platform_join() {
    let body = serde_json::json!({"scope_path": "general", "participant": "dave"});
    assert_conflict(
        read_only_app(),
        Request::builder()
            .extension(loopback())
            .method("POST")
            .uri("/platform/join")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
    )
    .await;
}

#[tokio::test]
async fn read_only_refuses_platform_roster() {
    let body = serde_json::json!({"scope_path": "general", "roster": ["dave"]});
    assert_conflict(
        read_only_app(),
        Request::builder()
            .extension(loopback())
            .method("POST")
            .uri("/platform/roster")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
    )
    .await;
}

#[tokio::test]
async fn read_only_refuses_platform_write_context() {
    let body = serde_json::json!({"scope_path": "general", "entries": []});
    assert_conflict(
        read_only_app(),
        Request::builder()
            .extension(loopback())
            .method("POST")
            .uri("/platform/context")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
    )
    .await;
}

#[tokio::test]
async fn read_only_refuses_platform_project() {
    let body = serde_json::json!({
        "target": {"participant": {"id": "dave"}},
        "attributes": [],
    });
    assert_conflict(
        read_only_app(),
        Request::builder()
            .extension(loopback())
            .method("POST")
            .uri("/platform/project")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
    )
    .await;
}

#[tokio::test]
async fn read_only_refuses_platform_message_stream() {
    // `message_stream` returns `Result<impl IntoResponse, ApiError>` rather than `Result<Json<T>,
    // ApiError>`, so its `refuse_if_read_only` early-return path is structurally different — this
    // test confirms the `ApiError` short-circuits before the SSE stream is built.
    let body = serde_json::json!({
        "scope_path": "general",
        "messages": [{ "sender": "dave", "text": "hello" }],
        "present": ["dave"],
    });
    assert_conflict(
        read_only_app(),
        Request::builder()
            .extension(loopback())
            .method("POST")
            .uri("/platform/messages/stream")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
    )
    .await;
}

#[tokio::test]
async fn read_only_refuses_platform_link() {
    let body = serde_json::json!({
        "from": {"participant": {"id": "dave"}},
        "to": {"context": {"scope_path": "general"}},
        "relation": "part_of",
    });
    assert_conflict(
        read_only_app(),
        Request::builder()
            .extension(loopback())
            .method("POST")
            .uri("/platform/link")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
    )
    .await;
}

// --- Platform surface: read handlers return 200 ---

#[tokio::test]
async fn read_only_serves_platform_self_memory() {
    assert_ok(
        read_only_app(),
        Request::builder()
            .extension(loopback())
            .uri("/platform/self")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
}
