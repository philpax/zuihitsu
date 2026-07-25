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

/// POST one context descriptor to `/platform/context` over loopback (scoped to `direct`), returning the
/// decoded `{ memory_id, entries }` outcome. The `supersedes` id, when present, revises the prior
/// descriptor in place.
async fn write_context(
    app: &axum::Router,
    text: &str,
    supersedes: Option<&str>,
) -> serde_json::Value {
    let body = serde_json::json!({
        "scope_path": "general",
        "entries": [{ "text": text, "supersedes": supersedes }],
    });
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .extension(loopback())
                .method("POST")
                .uri("/platform/context")
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
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn a_context_write_returns_the_memory_and_entry_ids() {
    // A first context write lands the descriptor and reports the context memory it minted plus the new
    // entry id, which the connector holds to supersede on the next change.
    let server =
        Arc::new(Server::in_memory(Box::new(ManualClock::new(Timestamp::from_millis(0)))).unwrap());
    let app = router(test_state(server.clone()));

    let outcome = write_context(&app, "Channel: Acme / general. Topic: none.", None).await;
    let expected_memory = server
        .control()
        .memory("context/direct:general")
        .unwrap()
        .unwrap()
        .id;
    assert_eq!(
        outcome["memory_id"],
        serde_json::json!(expected_memory.0.to_string())
    );
    let entries = outcome["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert!(entries[0].as_str().is_some_and(|id| !id.is_empty()));

    // Exactly one live entry, carrying the text written.
    let live = server.control().entries("context/direct:general").unwrap();
    assert_eq!(live.len(), 1);
    assert_eq!(live[0].text, "Channel: Acme / general. Topic: none.");
}

#[tokio::test]
async fn a_superseding_context_write_leaves_one_live_entry() {
    // A second write naming the first entry's id supersedes it, so the descriptor is revised in place —
    // exactly one live entry remains, carrying the new text. This is the restart-with-changed-metadata
    // path: without the supersede the two would stack.
    let server =
        Arc::new(Server::in_memory(Box::new(ManualClock::new(Timestamp::from_millis(0)))).unwrap());
    let app = router(test_state(server.clone()));

    let first = write_context(&app, "Channel: Acme / general.", None).await;
    let first_id = first["entries"][0].as_str().unwrap().to_owned();

    write_context(&app, "Channel: Acme / chat.", Some(&first_id)).await;

    let live = server.control().entries("context/direct:general").unwrap();
    assert_eq!(live.len(), 1);
    assert_eq!(live[0].text, "Channel: Acme / chat.");
}

#[tokio::test]
async fn a_supersede_of_a_dropped_entry_still_lands_the_fresh_append() {
    // A supersede whose target is no longer live — the connector held a stale id across a restart, and
    // the agent has since dropped that entry — is a no-op, but the fresh append still stands. Modelled
    // by superseding an entry a prior write already superseded: it is no longer live, so `supersede_if_live`
    // treats it as an unknown entry, exactly as an agent-side retraction would.
    let server =
        Arc::new(Server::in_memory(Box::new(ManualClock::new(Timestamp::from_millis(0)))).unwrap());
    let app = router(test_state(server.clone()));

    // First write, then a second superseding it — the first entry is now superseded (not live).
    let first = write_context(&app, "Channel: Acme / general.", None).await;
    let first_id = first["entries"][0].as_str().unwrap().to_owned();
    write_context(&app, "Channel: Acme / chat.", Some(&first_id)).await;

    // A third write again names the already-superseded first id. The supersede is a no-op, but the
    // append lands regardless — the write never fails on a target that moved underneath it.
    let third = write_context(&app, "Channel: Acme / lounge.", Some(&first_id)).await;
    assert!(
        third["entries"][0]
            .as_str()
            .is_some_and(|id| !id.is_empty())
    );

    let live = server.control().entries("context/direct:general").unwrap();
    let texts: Vec<&str> = live.iter().map(|e| e.text.as_str()).collect();
    assert!(texts.contains(&"Channel: Acme / lounge."));
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
