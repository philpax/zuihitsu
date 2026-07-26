use crate::http_server::tests::*;
#[tokio::test]
async fn health_reports_genesis_status() {
    let server =
        Arc::new(Server::in_memory(Box::new(ManualClock::new(Timestamp::from_millis(0)))).unwrap());
    let app = router(test_state(server));
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
    // No agent created, so genesis is Empty; no model configured, so the transport health is null.
    // The server is not booted read-only, so read_only is false.
    assert_eq!(
        &bytes[..],
        br#"{"genesis":"Empty","model":null,"read_only":false}"#
    );
}

/// With a resilience-wrapped model in the state, `/control/health` reports the circuit's state and
/// last failure — the surface the console's degraded-backend banner polls.
#[tokio::test]
async fn health_reports_the_model_transport_circuit() {
    let server =
        Arc::new(Server::in_memory(Box::new(ManualClock::new(Timestamp::from_millis(0)))).unwrap());
    let backend = Arc::new(zuihitsu::RetryingModel::new(
        Arc::new(ScriptedModel::new([])),
        &zuihitsu::ResilienceConfig::default(),
    ));
    let app = router(AppState {
        model: Some(backend.clone()),
        backend: Some(backend),
        ..test_state(server)
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
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["model"]["circuit"], "closed");
    assert_eq!(body["model"]["consecutive_failures"], 0);
    assert_eq!(body["model"]["last_failure"], serde_json::Value::Null);
}

/// A read-only `AppState` reports `read_only: true` in the health response — the surface the
/// console's read-only banner polls.
#[tokio::test]
async fn health_reports_read_only_mode() {
    let server =
        Arc::new(Server::in_memory(Box::new(ManualClock::new(Timestamp::from_millis(0)))).unwrap());
    let app = router(test_state_read_only(server));
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
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["read_only"], serde_json::json!(true));
}
