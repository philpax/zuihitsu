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

#[tokio::test]
async fn unmerge_endpoint_retracts_a_merge_then_404s_when_nothing_to_retract() {
    use zuihitsu::{
        EventPayload, LinkPosture, LinkSource, MemoryId, Namespace, RelationName, Visibility,
    };

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
            EventPayload::memory_created(b, Namespace::Person.with_name("marcus@chat")),
            EventPayload::link_created(
                a,
                b,
                RelationName::SameAs,
                LinkPosture {
                    source: LinkSource::Operator,
                    told_by: None,
                    told_in: None,
                    visibility: Visibility::Public,
                },
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
    use zuihitsu::{
        EventPayload, LinkPosture, LinkSource, MemoryId, Namespace, RelationName, Visibility,
    };

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
                LinkPosture {
                    source: LinkSource::Operator,
                    told_by: None,
                    told_in: None,
                    visibility: Visibility::Public,
                },
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
        "scope_path": "general",
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_overlapping_streamed_messages_supersede_the_first() {
    // The SSE sibling of the supersession integration tests: two overlapping
    // `POST /platform/messages/stream` requests for one room. The gate model parks the first turn
    // mid-stream so the second batch's arrival supersedes it; the first request's stream must
    // terminate promptly with a normal `outcome` frame carrying `Superseded` (and an `abandoned`
    // progress frame before it), while the second ends with the winner's `Reply`.
    use futures_util::stream::{self, StreamExt as _};
    use std::sync::Arc;
    use tokio::{
        sync::{Notify, watch},
        time::timeout,
    };
    use zuihitsu::{
        GenerateDelta, GenerateRequest, GenerateResponse, GenerateStream, ModelClient, ModelError,
        Usage, stream_response,
    };

    const FIRST_MARK: &str = "VENUE-QUERY-4471";
    const SECOND_MARK: &str = "CORRECTION-8213";
    const FIRST_TEXT: &str = "summarise the venue please (VENUE-QUERY-4471)";
    const SECOND_TEXT: &str = "scratch that — CORRECTION-8213: the venue moved to the wharf.";
    const WAIT: std::time::Duration = std::time::Duration::from_secs(10);

    // Replies immediately once its prompt carries every marker (the successor, answering with
    // everything in context); otherwise parks inside the stream so the mid-stream select can cancel
    // it when the newer batch lands.
    struct SupersedeGate {
        markers: Vec<&'static str>,
        entered: watch::Sender<usize>,
        release: Arc<Notify>,
    }

    #[async_trait::async_trait]
    impl ModelClient for SupersedeGate {
        fn model_id(&self) -> &str {
            "supersede-gate"
        }

        async fn generate_stream(&self, request: &GenerateRequest) -> GenerateStream {
            let prompt: String = request
                .messages
                .iter()
                .map(|message| message.content.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            if self.markers.iter().all(|marker| prompt.contains(marker)) {
                return stream_response(Ok(GenerateResponse {
                    completion: Completion::Reply("Got it — folding that in.".to_owned()),
                    usage: Usage::default(),
                    reasoning: None,
                    finish_reason: Some("stop".to_owned()),
                }));
            }
            self.entered.send_modify(|count| *count += 1);
            let release = self.release.clone();
            let fragment = stream::once(async {
                Ok::<GenerateDelta, ModelError>(GenerateDelta::Reply("thinking ".to_owned()))
            });
            let terminal = stream::once(async move {
                release.notified().await;
                Ok(GenerateDelta::Finished(GenerateResponse {
                    completion: Completion::Reply("done".to_owned()),
                    usage: Usage::default(),
                    reasoning: None,
                    finish_reason: Some("stop".to_owned()),
                }))
            });
            Box::pin(fragment.chain(terminal))
        }
    }

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

    let gate = Arc::new(SupersedeGate {
        markers: vec![FIRST_MARK, SECOND_MARK],
        entered: watch::channel(0usize).0,
        release: Arc::new(Notify::new()),
    });
    let mut entered = gate.entered.subscribe();
    let model: Arc<dyn ModelClient> = gate.clone();
    let app = router(AppState {
        model: Some(model),
        ..test_state(server)
    });

    let request = |text: &str| {
        let message = serde_json::json!({
            "scope_path": "leads",
            "messages": [{ "sender": "dave", "text": text }],
            "present": ["dave"],
        });
        Request::builder()
            .extension(loopback())
            .method("POST")
            .uri("/platform/messages/stream")
            .header("content-type", "application/json")
            .body(Body::from(message.to_string()))
            .unwrap()
    };

    // Open the first stream; its turn parks mid-generation.
    let first = app.clone().oneshot(request(FIRST_TEXT)).await.unwrap();
    assert_eq!(first.status(), StatusCode::OK);
    timeout(WAIT, entered.wait_for(|count| *count >= 1))
        .await
        .expect("the first stream begins generating")
        .unwrap();

    // Open the second stream for the same room: its arrival supersedes the first.
    let second = app.clone().oneshot(request(SECOND_TEXT)).await.unwrap();
    assert_eq!(second.status(), StatusCode::OK);

    let body1 = timeout(WAIT, axum::body::to_bytes(first.into_body(), usize::MAX))
        .await
        .expect("the superseded stream ends promptly")
        .unwrap();
    let body2 = timeout(WAIT, axum::body::to_bytes(second.into_body(), usize::MAX))
        .await
        .expect("the winner's stream ends after its reply")
        .unwrap();
    let text1 = String::from_utf8_lossy(&body1);
    let text2 = String::from_utf8_lossy(&body2);

    // The superseded stream carries an abandoned progress frame, then terminates with a Superseded
    // outcome frame — no reply, well before the successor finishes.
    let abandoned_at = text1
        .find("\"kind\":\"abandoned\"")
        .unwrap_or_else(|| panic!("the superseded stream carries an abandoned frame: {text1}"));
    let outcome1_at = text1
        .find("\"type\":\"outcome\"")
        .unwrap_or_else(|| panic!("the superseded stream ends with an outcome frame: {text1}"));
    assert!(
        text1[outcome1_at..].contains("Superseded"),
        "the first stream's terminal outcome is Superseded: {text1}"
    );
    assert!(
        abandoned_at < outcome1_at,
        "the abandoned frame precedes the terminal outcome: {text1}"
    );

    // The winner's stream ends with a reply outcome.
    let outcome2_at = text2
        .find("\"type\":\"outcome\"")
        .unwrap_or_else(|| panic!("the winner's stream ends with an outcome frame: {text2}"));
    assert!(
        text2[outcome2_at..].contains("Reply"),
        "the second stream's terminal outcome is a reply: {text2}"
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
        "scope_path": "general",
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
