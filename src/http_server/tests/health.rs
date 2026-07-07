use super::*;
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
    assert_eq!(&bytes[..], br#"{"genesis":"Empty","model":null}"#);
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
