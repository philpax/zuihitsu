//! HTTP tests for the `/platform/*` surface: the reserved `self` id, participant turns, connector
//! key scoping, roster resync, and the recorded model interactions.

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
use zuihitsu::{Completion, ManualClock, ModelCall, ScriptedModel, Server, time::Timestamp};

#[tokio::test]
async fn the_platform_self_endpoint_returns_the_reserved_self_memory_id() {
    // A born agent mints `self` at genesis; `GET /platform/self` reports its id, so a connector can
    // splice a `[mem:<id>]` reference for the agent's own @mention.
    let server = Server::in_memory(Box::new(ManualClock::new(Timestamp::from_millis(0)))).unwrap();
    server
        .control()
        .create_agent(&zuihitsu::SeedSelf {
            agent_name: "Kestrel".to_owned(),
            persona: "An assistant.".to_owned(),
            seed_entries: vec![],
        })
        .unwrap();
    let expected = server.control().memory("self").unwrap().unwrap().id;
    let server = Arc::new(server);
    let app = router(test_state(server));

    let response = app
        .oneshot(
            Request::builder()
                .extension(loopback())
                .uri("/platform/self")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["memory_id"], serde_json::json!(expected.0.to_string()));
}

#[tokio::test]
async fn a_platform_message_runs_a_turn() {
    // A born agent with a scripted model in app state: a /platform/messages delivers a participant
    // turn and returns the agent's reply.
    let server = Server::in_memory(Box::new(ManualClock::new(Timestamp::from_millis(0)))).unwrap();
    server
        .control()
        .create_agent(&zuihitsu::SeedSelf {
            agent_name: "Kestrel".to_owned(),
            persona: "An assistant.".to_owned(),
            seed_entries: vec![],
        })
        .unwrap();
    let model: Arc<dyn zuihitsu::ModelClient> = Arc::new(ScriptedModel::new([Completion::Reply(
        "Hi there.".to_owned(),
    )]));
    let app = router(AppState {
        model: Some(model),
        ..test_state(Arc::new(server))
    });

    let body = serde_json::json!({
        "scope_path": "general",
        "messages": [{ "sender": "dave", "text": "hello" }],
        "present": ["dave"],
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
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let response: zuihitsu::PlatformResponse = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        response.outcome,
        zuihitsu::TurnOutcome::Reply("Hi there.".to_owned())
    );
    assert!(
        !response.participant_turn_ids.is_empty() && !response.participant_turn_ids[0].is_empty()
    );
}

#[tokio::test]
async fn a_connector_key_scopes_a_write_to_its_own_platform() {
    // A connector on the same host as the server connects over loopback, yet its key — not its loopback
    // origin — decides its platform: its writes must land under its own platform, never mistaken for
    // the operator's `direct` interface. Regression for the loopback-first scoping bug.
    let server =
        Arc::new(Server::in_memory(Box::new(ManualClock::new(Timestamp::from_millis(0)))).unwrap());
    let platform_connectors: Arc<[(String, String)]> =
        Arc::from([("discord".to_owned(), "discord-key".to_owned())]);
    let app = router(AppState {
        platform_connectors,
        ..test_state(server.clone())
    });
    let response = app
        .oneshot(
            Request::builder()
                .extension(loopback())
                .method("POST")
                .uri("/platform/project")
                .header("authorization", "Bearer discord-key")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"target":{"participant":{"id":"dave"}},"attributes":[{"text":"Discord username: dave1234","supersedes":null}]}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    // The write landed on the discord-qualified stub the key scopes to, not a direct one.
    assert!(
        server
            .control()
            .memory("person/dave@discord")
            .unwrap()
            .is_some()
    );
    assert!(
        server
            .control()
            .memory("person/dave@direct")
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn a_platform_roster_resync_briefs_arrivals_and_reports_departures() {
    // A born agent with a scripted model: a /platform/messages opens a session with Dave present,
    // then a /platform/roster resync brings Erin in and drops Dave, returning the diff as JSON.
    let server = Server::in_memory(Box::new(ManualClock::new(Timestamp::from_millis(0)))).unwrap();
    server
        .control()
        .create_agent(&zuihitsu::SeedSelf {
            agent_name: "Kestrel".to_owned(),
            persona: "An assistant.".to_owned(),
            seed_entries: vec![],
        })
        .unwrap();
    let model: Arc<dyn zuihitsu::ModelClient> = Arc::new(ScriptedModel::new([Completion::Reply(
        "Hi there.".to_owned(),
    )]));
    let app = router(AppState {
        model: Some(model),
        ..test_state(Arc::new(server))
    });

    let message = serde_json::json!({
        "scope_path": "general",
        "messages": [{ "sender": "dave", "text": "hello" }],
        "present": ["dave"],
    });
    app.clone()
        .oneshot(
            Request::builder()
                .extension(loopback())
                .method("POST")
                .uri("/platform/messages")
                .header("content-type", "application/json")
                .body(Body::from(message.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    let resync = serde_json::json!({
        "scope_path": "general",
        "roster": ["erin"],
    });
    let response = app
        .oneshot(
            Request::builder()
                .extension(loopback())
                .method("POST")
                .uri("/platform/roster")
                .header("content-type", "application/json")
                .body(Body::from(resync.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    // Erin arrived (briefed in); Dave, absent from the roster, is the one prior member reported as
    // departed.
    assert_eq!(&bytes[..], br#"{"joined":["erin"],"departed":1}"#);
}

#[tokio::test]
async fn interactions_surface_the_recorded_model_calls() {
    // After a scripted turn, `/control/interactions` returns the model-interaction record.
    let server = Server::in_memory(Box::new(ManualClock::new(Timestamp::from_millis(0)))).unwrap();
    server
        .control()
        .create_agent(&zuihitsu::SeedSelf {
            agent_name: "Kestrel".to_owned(),
            persona: "An assistant.".to_owned(),
            seed_entries: vec![],
        })
        .unwrap();
    let model: Arc<dyn zuihitsu::ModelClient> = Arc::new(ScriptedModel::new([Completion::Reply(
        "Hi there.".to_owned(),
    )]));
    let app = router(AppState {
        model: Some(model),
        ..test_state(Arc::new(server))
    });

    let body = serde_json::json!({
        "scope_path": "general",
        "messages": [{ "sender": "dave", "text": "hello" }],
        "present": ["dave"],
    });
    app.clone()
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

    let response = app
        .oneshot(
            Request::builder()
                .extension(loopback())
                .uri("/control/interactions")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let calls: Vec<ModelCall> = serde_json::from_slice(&bytes).unwrap();
    // The single reply step was recorded, with its completion and a non-empty digest.
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].completion,
        Completion::Reply("Hi there.".to_owned())
    );
    assert!(!calls[0].request_digest.is_empty());
}
