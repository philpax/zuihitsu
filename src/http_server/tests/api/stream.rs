//! HTTP tests for the SSE event stream and the streamed message endpoint: the opening snapshot, the
//! live tail, progress-then-outcome framing, overlapping-message supersession, and shutdown.

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
use zuihitsu::{Completion, ManualClock, ScriptedModel, Server, time::Timestamp};

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
