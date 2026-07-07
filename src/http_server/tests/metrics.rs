use super::*;

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
        model: Some(model),
        metrics: Some(recorder.handle()),
        ..test_state(Arc::new(server))
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
    let app = router(test_state(Arc::new(server)));
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
