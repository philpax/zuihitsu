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
        "locator": { "platform": "discord", "scope_path": "general" },
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
        "locator": { "platform": "discord", "scope_path": "general" },
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
        "locator": { "platform": "discord", "scope_path": "general" },
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
        "locator": { "platform": "discord", "scope_path": "general" },
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

#[tokio::test]
async fn unmerge_endpoint_retracts_a_merge_then_404s_when_nothing_to_retract() {
    use zuihitsu::{EventPayload, LinkSource, MemoryId, Namespace, RelationName, Visibility};

    let server = Server::in_memory(Box::new(ManualClock::new(Timestamp::from_millis(0)))).unwrap();
    server
        .control()
        .create_agent(&zuihitsu::SeedSelf {
            agent_name: "Kestrel".to_owned(),
            persona: "An assistant.".to_owned(),
            seed_entries: vec![],
        })
        .unwrap();
    let a = MemoryId::generate();
    let b = MemoryId::generate();
    server
        .control()
        .seed_events(vec![
            EventPayload::memory_created(a, Namespace::Person.with_name("marcus@direct")),
            EventPayload::memory_created(b, Namespace::Person.with_name("marcus@discord")),
            EventPayload::link_created(
                a,
                b,
                RelationName::SameAs,
                LinkSource::Operator,
                None,
                None,
                Visibility::Public,
            ),
        ])
        .unwrap();
    let app = router(test_state(Arc::new(server)));

    let post = |from: MemoryId, to: MemoryId| {
        let body = serde_json::json!({ "from": from.0.to_string(), "to": to.0.to_string() });
        Request::builder()
            .extension(loopback())
            .method("POST")
            .uri("/control/unmerge")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    };

    // The first retraction removes the `same_as` edge.
    let removed = app.clone().oneshot(post(a, b)).await.unwrap();
    assert_eq!(removed.status(), StatusCode::NO_CONTENT);

    // A second retraction finds nothing directly merged — 404.
    let again = app.oneshot(post(a, b)).await.unwrap();
    assert_eq!(again.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn designate_primary_endpoint_pins_a_stub_then_404s_on_an_unknown_memory() {
    use zuihitsu::{EventPayload, LinkSource, MemoryId, Namespace, RelationName, Visibility};

    let server = Server::in_memory(Box::new(ManualClock::new(Timestamp::from_millis(0)))).unwrap();
    server
        .control()
        .create_agent(&zuihitsu::SeedSelf {
            agent_name: "Kestrel".to_owned(),
            persona: "An assistant.".to_owned(),
            seed_entries: vec![],
        })
        .unwrap();
    let mut ids = [MemoryId::generate(), MemoryId::generate()];
    ids.sort();
    let (older, newer) = (ids[0], ids[1]);
    server
        .control()
        .seed_events(vec![
            EventPayload::memory_created(older, Namespace::Person.with_name("pat")),
            EventPayload::memory_created(newer, Namespace::Person.with_name("patricia")),
            EventPayload::link_created(
                older,
                newer,
                RelationName::SameAs,
                LinkSource::Operator,
                None,
                None,
                Visibility::Public,
            ),
        ])
        .unwrap();
    let app = router(test_state(Arc::new(server)));

    let post = |memory: MemoryId, designated: bool| {
        let body = serde_json::json!({ "memory": memory.0.to_string(), "designated": designated });
        Request::builder()
            .extension(loopback())
            .method("POST")
            .uri("/control/designate-primary")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    };

    // Pinning the later-minted stub succeeds.
    let pinned = app.clone().oneshot(post(newer, true)).await.unwrap();
    assert_eq!(pinned.status(), StatusCode::NO_CONTENT);

    // A designation naming no live memory is a 404.
    let ghost = MemoryId::generate();
    let missing = app.oneshot(post(ghost, true)).await.unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
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

#[tokio::test]
async fn self_edit_endpoint_appends_revises_and_validates() {
    // A born agent whose `self` carries only its seeded persona entry.
    let server = Server::in_memory(Box::new(ManualClock::new(Timestamp::from_millis(0)))).unwrap();
    server
        .control()
        .create_agent(&zuihitsu::SeedSelf {
            agent_name: "Kestrel".to_owned(),
            persona: "A discreet companion with a long memory.".to_owned(),
            seed_entries: vec![],
        })
        .unwrap();
    let app = router(test_state(Arc::new(server)));

    let edit = |body: serde_json::Value| {
        app.clone().oneshot(
            Request::builder()
                .extension(loopback())
                .method("POST")
                .uri("/control/self")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
    };

    // An append returns 200 and the new entry id.
    let appended = edit(serde_json::json!({ "text": "I keep Marcus's memory." }))
        .await
        .unwrap();
    assert_eq!(appended.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(appended.into_body(), usize::MAX)
        .await
        .unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let appended_id = value["entry_id"].as_str().unwrap().to_owned();
    assert!(!appended_id.is_empty(), "the response names the new entry");

    // The seeded persona entry's id, read back through the entries endpoint, to revise it.
    let entries = app
        .clone()
        .oneshot(
            Request::builder()
                .extension(loopback())
                .uri("/control/entries?name=self")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = axum::body::to_bytes(entries.into_body(), usize::MAX)
        .await
        .unwrap();
    let entries: Vec<serde_json::Value> = serde_json::from_slice(&bytes).unwrap();
    let persona_id = entries[0]["entry_id"].as_str().unwrap().to_owned();

    // A revision (supersedes the persona entry) returns 200.
    let revised = edit(serde_json::json!({
        "text": "A discreet companion who keeps Marcus's memory.",
        "supersedes": persona_id,
    }))
    .await
    .unwrap();
    assert_eq!(revised.status(), StatusCode::OK);

    // An empty edit is a 400.
    let empty = edit(serde_json::json!({ "text": "   " })).await.unwrap();
    assert_eq!(empty.status(), StatusCode::BAD_REQUEST);

    // Superseding an unknown entry is a 404.
    let ghost = zuihitsu::EntryId::generate();
    let unknown = edit(serde_json::json!({ "text": "replacement", "supersedes": ghost }))
        .await
        .unwrap();
    assert_eq!(unknown.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_event_stream_opens_with_the_committed_snapshot() {
    // A born agent has genesis events; the stream's first frames replay them as `event` records
    // before the live tail begins. The stream never ends on its own (keep-alive), so the test reads
    // the first body chunk and asserts its shape rather than draining to completion.
    let server = Server::in_memory(Box::new(ManualClock::new(Timestamp::from_millis(0)))).unwrap();
    server
        .control()
        .create_agent(&zuihitsu::SeedSelf {
            agent_name: "Kestrel".to_owned(),
            persona: "An assistant.".to_owned(),
            seed_entries: vec![],
        })
        .unwrap();
    let app = router(test_state(Arc::new(server)));

    let response = app
        .oneshot(
            Request::builder()
                .extension(loopback())
                .method("GET")
                .uri("/control/events/stream?from=0")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.starts_with("text/event-stream"))
    );

    use futures_util::StreamExt as _;
    let mut frames = response.into_body().into_data_stream();
    let first = tokio::time::timeout(std::time::Duration::from_secs(5), frames.next())
        .await
        .expect("the snapshot arrives promptly")
        .expect("the stream is open")
        .expect("the frame reads");
    let text = String::from_utf8_lossy(&first);
    assert!(
        text.contains("\"type\":\"event\""),
        "the first frames are committed events, got: {text}"
    );
    assert!(
        text.contains("\"seq\":1") && text.contains("\"source\":\"Bootstrap\""),
        "the snapshot replays the log from seq 1 with its envelope source, got: {text}"
    );
}

#[tokio::test]
async fn a_streamed_platform_message_yields_progress_then_the_outcome() {
    // The streamed sibling of `/platform/messages`: reply tokens arrive as `progress` frames while
    // the turn runs, and the terminal `outcome` frame carries the same TurnOutcome the unary
    // endpoint would return. The scripted model streams word fragments, so the frames are real.
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
        "Hello there, Dave.".to_owned(),
    )]));
    let app = router(AppState {
        model: Some(model),
        ..test_state(Arc::new(server))
    });

    let message = serde_json::json!({
        "locator": { "platform": "discord", "scope_path": "general" },
        "messages": [{ "sender": "dave", "text": "hello" }],
        "present": ["dave"],
    });
    let response = app
        .oneshot(
            Request::builder()
                .extension(loopback())
                .method("POST")
                .uri("/platform/messages/stream")
                .header("content-type", "application/json")
                .body(Body::from(message.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    // The stream ends itself after the terminal frame, so the whole body is finite and readable.
    let bytes = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        axum::body::to_bytes(response.into_body(), usize::MAX),
    )
    .await
    .expect("the stream ends after the outcome")
    .unwrap();
    let text = String::from_utf8_lossy(&bytes);
    assert!(
        text.contains("\"type\":\"progress\""),
        "progress frames arrive: {text}"
    );
    assert!(
        text.contains("\"kind\":\"reply\"") && text.contains("Hello "),
        "the reply streams as fragments: {text}"
    );
    let outcome_at = text
        .find("\"type\":\"outcome\"")
        .expect("the terminal outcome frame arrives");
    assert!(
        text[outcome_at..].contains("Hello there, Dave."),
        "the outcome carries the whole reply: {text}"
    );
}

#[tokio::test]
async fn the_event_stream_pushes_the_live_tail_and_progress_frames() {
    // Beyond the snapshot: a turn driven after the stream opens pushes its committed events, and
    // the generation's ephemeral progress frames ride through as their own frame type — the full
    // push channel, exercised by a real scripted turn rather than a synthetic publish.
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
    let model: Arc<dyn zuihitsu::ModelClient> = Arc::new(ScriptedModel::new([Completion::Reply(
        "Hello there.".to_owned(),
    )]));
    let app = router(AppState {
        model: Some(model),
        ..test_state(server.clone())
    });

    // Opening just past the current head skips the whole snapshot (the genesis events), so
    // everything read below was pushed live. The `from` horizon is honoured exactly: a tail
    // event below it would be withheld, so an inflated horizon would hang the read loop.
    let head = server
        .control()
        .events()
        .unwrap()
        .last()
        .map(|event| event.seq.0)
        .unwrap_or_default();
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .extension(loopback())
                .method("GET")
                .uri(format!("/control/events/stream?from={}", head + 1))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    use futures_util::StreamExt as _;
    let mut frames = response.into_body().into_data_stream();

    let message = serde_json::json!({
        "locator": { "platform": "discord", "scope_path": "general" },
        "messages": [{ "sender": "dave", "text": "hello" }],
        "present": ["dave"],
    });
    app.oneshot(
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

    // The turn's progress frames and its committed events both arrive over the one stream.
    let mut collected = String::new();
    while !(collected.contains("\"type\":\"progress\"") && collected.contains("ConversationTurn")) {
        let chunk = tokio::time::timeout(std::time::Duration::from_secs(5), frames.next())
            .await
            .expect("the pushed frames arrive")
            .expect("the stream is open")
            .expect("the frame reads");
        collected.push_str(&String::from_utf8_lossy(&chunk));
    }
    assert!(collected.contains("\"type\":\"event\""));
    assert!(collected.contains("\"kind\":\"reply\""));
}

#[tokio::test]
async fn the_event_stream_ends_when_shutdown_is_signalled() {
    // The SSE loop has no feed that closes on its own, so it must end when the shutdown flag fires —
    // otherwise `with_graceful_shutdown` waits on the open connection forever and the server never
    // exits (the deadlock this arm fixes). Open the stream, read its snapshot, fire shutdown, and
    // assert the body then completes rather than hanging on its now-idle feeds.
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
    let (shutdown, fire) = crate::http_server::console::ShutdownFlag::controllable();
    let app = router(AppState {
        shutdown,
        ..test_state(server)
    });

    let response = app
        .oneshot(
            Request::builder()
                .extension(loopback())
                .method("GET")
                .uri("/control/events/stream?from=0")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    use futures_util::StreamExt as _;
    let mut frames = response.into_body().into_data_stream();
    // The committed snapshot arrives first, ahead of the tail loop the shutdown must break.
    tokio::time::timeout(std::time::Duration::from_secs(5), frames.next())
        .await
        .expect("the snapshot arrives promptly")
        .expect("the stream is open")
        .expect("the frame reads");

    // Fire shutdown: the tail loop must break and the body complete, rather than blocking forever on
    // feeds that never close. Without the shutdown arm this drain never finishes.
    fire.send(true).unwrap();
    let drained = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while frames.next().await.is_some() {}
    })
    .await;
    assert!(
        drained.is_ok(),
        "the stream ends after shutdown is signalled rather than hanging"
    );
}
