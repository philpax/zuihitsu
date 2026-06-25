use super::{AppState, router};
use axum::{
    body::Body,
    extract::ConnectInfo,
    http::{Request, StatusCode},
};
use std::{net::SocketAddr, sync::Arc};
use tower::ServiceExt;
use zuihitsu::{
    Completion, ManualClock, ModelCall, ScriptedModel, Server,
    metrics::{LATENCY_BUCKETS, describe},
    time::Timestamp,
};

/// No configured keys — the existing tests run loopback, where keys are not consulted.
fn no_keys() -> Arc<[String]> {
    Vec::new().into()
}

/// A loopback peer extension to inject into a `oneshot` request (real `axum::serve` sets this from
/// the socket; `Request::builder()` does not). The auth middleware trusts a loopback peer, so the
/// existing assertions are unaffected.
fn loopback() -> ConnectInfo<SocketAddr> {
    ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 0)))
}

/// A non-loopback peer extension, for the auth tests — a remote peer must present a valid key.
fn remote() -> ConnectInfo<SocketAddr> {
    ConnectInfo(SocketAddr::from(([203, 0, 113, 1], 1234)))
}

/// The router serves `/control/health` over an in-memory server, with no real socket — `oneshot`
/// drives one request through the tower service.
#[tokio::test]
async fn health_reports_genesis_status() {
    let server =
        Arc::new(Server::in_memory(Box::new(ManualClock::new(Timestamp::from_millis(0)))).unwrap());
    let app = router(AppState {
        server,
        model: None,
        snapshot_dir: None,
        metrics: None,
        boot: std::time::Instant::now(),
        control_keys: no_keys(),
        platform_keys: no_keys(),
        config: Arc::new(zuihitsu::EnvConfig::default()),
    });
    let response = app
        .oneshot(
            Request::builder()
                .extension(loopback())
                .uri("/control/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    // No agent created, so genesis is Empty.
    assert_eq!(&bytes[..], br#"{"genesis":"Empty"}"#);
}

#[tokio::test]
async fn create_then_inspect_over_the_control_api() {
    let server =
        Arc::new(Server::in_memory(Box::new(ManualClock::new(Timestamp::from_millis(0)))).unwrap());
    let app = router(AppState {
        server,
        model: None,
        snapshot_dir: None,
        metrics: None,
        boot: std::time::Instant::now(),
        control_keys: no_keys(),
        platform_keys: no_keys(),
        config: Arc::new(zuihitsu::EnvConfig::default()),
    });

    // Create the agent through the API.
    let seed = serde_json::json!({
        "agent_name": "Kestrel",
        "persona": "An assistant.",
        "seed_entries": [],
    });
    let created = app
        .clone()
        .oneshot(
            Request::builder()
                .extension(loopback())
                .method("POST")
                .uri("/control/agent")
                .header("content-type", "application/json")
                .body(Body::from(seed.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::OK);

    // Genesis now reports Complete.
    let genesis = app
        .clone()
        .oneshot(
            Request::builder()
                .extension(loopback())
                .uri("/control/genesis")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = axum::body::to_bytes(genesis.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(&bytes[..], br#""Complete""#);

    // `self` exists; an unknown memory is a 404.
    let self_memory = app
        .clone()
        .oneshot(
            Request::builder()
                .extension(loopback())
                .uri("/control/memory?name=self")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(self_memory.status(), StatusCode::OK);

    let missing = app
        .oneshot(
            Request::builder()
                .extension(loopback())
                .uri("/control/memory?name=person/nobody")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_platform_message_runs_a_turn() {
    // A born agent with a scripted model in app state: a /platform/message delivers a participant
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
        server: Arc::new(server),
        model: Some(model),
        snapshot_dir: None,
        metrics: None,
        boot: std::time::Instant::now(),
        control_keys: no_keys(),
        platform_keys: no_keys(),
        config: Arc::new(zuihitsu::EnvConfig::default()),
    });

    let body = serde_json::json!({
        "locator": { "platform": "discord", "scope_path": "general" },
        "sender": "dave",
        "text": "hello",
        "present": ["dave"],
    });
    let response = app
        .oneshot(
            Request::builder()
                .extension(loopback())
                .method("POST")
                .uri("/platform/message")
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
    assert_eq!(&bytes[..], br#"{"Reply":"Hi there."}"#);
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
        server: Arc::new(server),
        model: Some(model),
        snapshot_dir: None,
        metrics: None,
        boot: std::time::Instant::now(),
        control_keys: no_keys(),
        platform_keys: no_keys(),
        config: Arc::new(zuihitsu::EnvConfig::default()),
    });

    let body = serde_json::json!({
        "locator": { "platform": "discord", "scope_path": "general" },
        "sender": "dave",
        "text": "hello",
        "present": ["dave"],
    });
    app.clone()
        .oneshot(
            Request::builder()
                .extension(loopback())
                .method("POST")
                .uri("/platform/message")
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

#[tokio::test]
async fn snapshot_endpoint_writes_a_file_or_409s_when_disabled() {
    let born = || {
        let server =
            Server::in_memory(Box::new(ManualClock::new(Timestamp::from_millis(0)))).unwrap();
        server
            .control()
            .create_agent(&zuihitsu::SeedSelf {
                agent_name: "Kestrel".to_owned(),
                persona: "An assistant.".to_owned(),
                seed_entries: vec![],
            })
            .unwrap();
        server
    };
    let post = || {
        Request::builder()
            .extension(loopback())
            .method("POST")
            .uri("/control/snapshot")
            .body(Body::empty())
            .unwrap()
    };

    // Enabled: the endpoint writes a snapshot into the configured directory.
    let dir = std::env::temp_dir().join(format!(
        "zuihitsu-snapep-{}",
        zuihitsu::MemoryId::generate().0
    ));
    let app = router(AppState {
        server: Arc::new(born()),
        model: None,
        snapshot_dir: Some(dir.clone()),
        metrics: None,
        boot: std::time::Instant::now(),
        control_keys: no_keys(),
        platform_keys: no_keys(),
        config: Arc::new(zuihitsu::EnvConfig::default()),
    });
    let response = app.oneshot(post()).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(zuihitsu::snapshot::latest(&dir).unwrap().is_some());
    std::fs::remove_dir_all(&dir).unwrap();

    // Disabled (no snapshot dir): the endpoint answers 409.
    let app = router(AppState {
        server: Arc::new(born()),
        model: None,
        snapshot_dir: None,
        metrics: None,
        boot: std::time::Instant::now(),
        control_keys: no_keys(),
        platform_keys: no_keys(),
        config: Arc::new(zuihitsu::EnvConfig::default()),
    });
    let response = app.oneshot(post()).await.unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
}

/// A router over a fresh in-memory server with the given per-surface keys — for the auth tests.
fn keyed_app(control: &[&str], platform: &[&str]) -> axum::Router {
    let server =
        Arc::new(Server::in_memory(Box::new(ManualClock::new(Timestamp::from_millis(0)))).unwrap());
    let keys = |k: &[&str]| -> Arc<[String]> { k.iter().map(|s| s.to_string()).collect() };
    router(AppState {
        server,
        model: None,
        snapshot_dir: None,
        metrics: None,
        boot: std::time::Instant::now(),
        control_keys: keys(control),
        platform_keys: keys(platform),
        config: Arc::new(zuihitsu::EnvConfig::default()),
    })
}

/// A GET request from `peer`, optionally bearing `key`.
fn get(peer: ConnectInfo<SocketAddr>, uri: &str, key: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder().extension(peer).uri(uri);
    if let Some(key) = key {
        builder = builder.header("authorization", format!("Bearer {key}"));
    }
    builder.body(Body::empty()).unwrap()
}

#[tokio::test]
async fn a_remote_peer_without_a_valid_key_is_rejected() {
    let app = keyed_app(&["op-key"], &["pf-key"]);
    // No Authorization header → 401, on both surfaces.
    let response = app
        .clone()
        .oneshot(get(remote(), "/control/genesis", None))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .extension(remote())
                .method("POST")
                .uri("/platform/message")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    // A wrong key → 401.
    let response = app
        .oneshot(get(remote(), "/control/genesis", Some("nope")))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_valid_key_authorizes_only_its_own_surface() {
    let app = keyed_app(&["op-key"], &["pf-key"]);
    // The control key opens a control route.
    let response = app
        .clone()
        .oneshot(get(remote(), "/control/genesis", Some("op-key")))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    // ...but the same key does NOT open a platform route — the surfaces are isolated.
    let response = app
        .oneshot(
            Request::builder()
                .extension(remote())
                .method("POST")
                .uri("/platform/message")
                .header("authorization", "Bearer op-key")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_loopback_peer_is_trusted_without_a_key() {
    // Even with keys configured, a loopback peer needs none — the local CLI keeps working.
    let app = keyed_app(&["op-key"], &["pf-key"]);
    let response = app
        .oneshot(get(loopback(), "/control/genesis", None))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn an_empty_key_list_is_fail_closed_for_remote_peers() {
    // No keys configured + a remote peer → always rejected, so a wide bind with no keys is a
    // silent lockout, never a silent exposure.
    let app = keyed_app(&[], &[]);
    let response = app
        .oneshot(get(remote(), "/control/genesis", None))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

/// Build an app over a born agent that has run one scripted turn, wired to a fresh local metrics
/// recorder so the `/control/metrics` scrape reads the turn's observations. The recorder and its
/// thread-local guard live in the caller so they span both the turn and the scrape (a guard scoped
/// to the turn alone would miss the gauge-refresh the handler runs at scrape time). Returns the app
/// ready for a `oneshot` GET.
async fn app_with_metrics_after_a_turn(
    recorder: &metrics_exporter_prometheus::PrometheusRecorder,
) -> axum::Router {
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
        server: Arc::new(server),
        model: Some(model),
        snapshot_dir: None,
        metrics: Some(recorder.handle()),
        boot: std::time::Instant::now(),
        control_keys: no_keys(),
        platform_keys: no_keys(),
        config: Arc::new(zuihitsu::EnvConfig::default()),
    });
    let body = serde_json::json!({
        "locator": { "platform": "discord", "scope_path": "general" },
        "sender": "dave",
        "text": "hello",
        "present": ["dave"],
    });
    app.clone()
        .oneshot(
            Request::builder()
                .extension(loopback())
                .method("POST")
                .uri("/platform/message")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    app
}

#[tokio::test]
async fn metrics_endpoint_renders_prometheus_text_after_a_turn() {
    // A local recorder (not the global) keeps the test isolated; its thread-local guard spans the
    // turn and the scrape so both the observations and the gauge-refresh land in this recorder.
    let recorder = metrics_exporter_prometheus::PrometheusBuilder::new()
        .set_buckets(LATENCY_BUCKETS)
        .unwrap()
        .build_recorder();
    let _guard = metrics::set_default_local_recorder(&recorder);
    describe();
    let app = app_with_metrics_after_a_turn(&recorder).await;
    let response = app
        .oneshot(
            Request::builder()
                .extension(loopback())
                .uri("/control/metrics")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get("content-type").unwrap(),
        "text/plain; version=0.0.4; charset=utf-8"
    );
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    // The four golden signals are declared (HELP/TYPE) and the turn's counts surface as samples.
    assert!(text.contains("# TYPE zuihitsu_turns_total counter\n"));
    assert!(
        text.contains("zuihitsu_turns_total 1\n"),
        "one turn observed"
    );
    assert!(text.contains("# TYPE zuihitsu_model_calls_total counter\n"));
    assert!(
        text.contains("zuihitsu_model_calls_total 1\n"),
        "the turn's step was observed at the chokepoint"
    );
    assert!(text.contains("# TYPE zuihitsu_turns_duration_seconds histogram\n"));
    assert!(text.contains("zuihitsu_turns_duration_seconds_count 1\n"));
    assert!(text.contains("# TYPE zuihitsu_sessions_active gauge\n"));
    assert!(text.contains("zuihitsu_sessions_active 1\n"));
    assert!(text.contains("# TYPE zuihitsu_head_seq gauge\n"));
    // The agent-state gauges are refreshed from the graph at scrape time.
    assert!(text.contains("# TYPE zuihitsu_memory_count gauge\n"));
    // The MCP gauges read zero (no servers configured) — a gauge set at scrape renders even at 0.
    assert!(text.contains("zuihitsu_mcp_servers_up 0\n"));
}

#[tokio::test]
async fn the_metrics_endpoint_is_503_without_a_recorder() {
    // No recorder installed (the AppState carries no handle) → 503, not a panic.
    let server = Server::in_memory(Box::new(ManualClock::new(Timestamp::from_millis(0)))).unwrap();
    let app = router(AppState {
        server: Arc::new(server),
        model: None,
        snapshot_dir: None,
        metrics: None,
        boot: std::time::Instant::now(),
        control_keys: no_keys(),
        platform_keys: no_keys(),
        config: Arc::new(zuihitsu::EnvConfig::default()),
    });
    let response = app
        .oneshot(
            Request::builder()
                .extension(loopback())
                .uri("/control/metrics")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}
