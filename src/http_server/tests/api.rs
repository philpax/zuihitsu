use super::*;

#[tokio::test]
async fn create_then_inspect_over_the_control_api() {
    let server =
        Arc::new(Server::in_memory(Box::new(ManualClock::new(Timestamp::from_millis(0)))).unwrap());
    let app = router(test_state(server));

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
        model: Some(model),
        ..test_state(Arc::new(server))
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
        model: Some(model),
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
        snapshot_dir: Some(dir.clone()),
        ..test_state(Arc::new(born()))
    });
    let response = app.oneshot(post()).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(zuihitsu::snapshot::latest(&dir).unwrap().is_some());
    std::fs::remove_dir_all(&dir).unwrap();

    // Disabled (no snapshot dir): the endpoint answers 409.
    let app = router(test_state(Arc::new(born())));
    let response = app.oneshot(post()).await.unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
}
