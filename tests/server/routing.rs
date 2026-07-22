use super::*;
#[tokio::test]
async fn the_indexer_catches_the_vector_index_up_to_the_log() {
    let embedder: std::sync::Arc<dyn Embedder> =
        std::sync::Arc::new(common::CpuEmbedder::try_new().unwrap());
    let vectors: Box<dyn VectorIndex> = Box::new(InMemoryVectorIndex::new());
    let mut server = Server::with_retrieval(
        Box::new(MemoryStore::new()),
        Graph::open_in_memory().unwrap(),
        clock(),
        embedder,
        vectors,
    );
    server.boot().unwrap();
    // Genesis writes the agent's self memory with seed content entries — the indexer's input.
    server.control().create_agent(&seed()).unwrap();

    // The first catch-up embeds the genesis content into the index and advances the cursor; a second
    // catch-up, with nothing new in the log, is a no-op — proving the cursor threads through the server.
    let indexed = server.index_catch_up().await.unwrap();
    assert!(
        indexed > 0,
        "genesis content should be indexed, got {indexed}"
    );
    assert_eq!(server.index_catch_up().await.unwrap(), 0);
}

/// A born agent over an in-memory store, returning a clock handle (sharing the boxed clock's atomic)
/// so a test can advance time.

#[tokio::test]
async fn route_message_opens_a_session_and_runs_a_turn() {
    let (server, _clock) = born_agent();
    let model = ScriptedModel::new([Completion::Reply("Hi, Dave.".to_owned())]);
    let leads = ConversationLocator::new(TEST_PLATFORM, "leads");

    let outcome = server
        .platform()
        .route_message(
            &model,
            &leads,
            &PersonId::new(TEST_PLATFORM, "dave"),
            "hello there",
            &[PersonId::new(TEST_PLATFORM, "dave")],
        )
        .await
        .unwrap();
    assert_eq!(outcome.outcome, TurnOutcome::Reply("Hi, Dave.".to_owned()));

    // First contact minted the room's context and the sender's stub.
    assert!(
        server
            .control()
            .memory("context/chat:leads")
            .unwrap()
            .is_some()
    );
    assert!(
        server
            .control()
            .memory("person/dave@chat")
            .unwrap()
            .is_some()
    );

    // One session opened, carrying a frozen, non-empty brief.
    let sessions = server.control().sessions(&leads).unwrap();
    assert_eq!(sessions.len(), 1);
    assert!(!sessions[0].brief.is_empty());
}

/// A model that observes how many turns reach inference at once. Each `generate` records the live
/// count's peak, then rendezvouses on a barrier sized to the stream limit, so exactly the limit's
/// worth of turns meet here before any proceeds — making the observed peak the limit, deterministically
/// (when driven with a whole number of barrier-sized waves).
struct ConcurrencyProbe {
    active: AtomicUsize,
    peak: AtomicUsize,
    barrier: tokio::sync::Barrier,
}

#[async_trait::async_trait]
impl ModelClient for ConcurrencyProbe {
    fn model_id(&self) -> &str {
        "concurrency-probe"
    }

    async fn generate_stream(&self, _request: &GenerateRequest) -> GenerateStream {
        let step: Result<GenerateResponse, ModelError> = async {
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(active, Ordering::SeqCst);
            // The permitted streams (== the limit) rendezvous here before any returns, so they are all
            // simultaneously "in flight" and the peak reflects the full limit.
            self.barrier.wait().await;
            self.active.fetch_sub(1, Ordering::SeqCst);
            Ok(GenerateResponse {
                completion: Completion::Reply("done".to_owned()),
                usage: Usage::default(),
                reasoning: None,
                finish_reason: None,
            })
        }
        .await;
        stream_response(step)
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_stream_limit_caps_concurrent_turns() {
    // The server sizes its stream semaphore from `ConcurrencySettings::default()` at construction.
    // Drive twice the limit's worth of concurrent messages — a whole number of barrier waves — through
    // one shared server; the semaphore must hold the peak concurrency at exactly the limit.
    let limit = ConcurrencySettings::default().max_concurrent_streams as usize;
    let waves = 2;
    let (server, _clock) = born_agent();
    let server = Arc::new(server);
    let probe = Arc::new(ConcurrencyProbe {
        active: AtomicUsize::new(0),
        peak: AtomicUsize::new(0),
        barrier: tokio::sync::Barrier::new(limit),
    });

    let mut tasks = Vec::new();
    for i in 0..(limit * waves) {
        let server = server.clone();
        let probe = probe.clone();
        tasks.push(tokio::spawn(async move {
            // A distinct room and sender per turn, so the turns mint disjoint stubs — this test
            // isolates the stream limit, not the same-memory contention the locks (2b) handle.
            let room = ConversationLocator::new(TEST_PLATFORM, format!("room-{i}"));
            let sender = PersonId::new(TEST_PLATFORM, format!("user-{i}"));
            server
                .platform()
                .route_message(
                    probe.as_ref(),
                    &room,
                    &sender,
                    "ping",
                    std::slice::from_ref(&sender),
                )
                .await
        }));
    }
    for task in tasks {
        let outcome = task.await.expect("the turn task joins").expect("turn runs");
        assert!(matches!(outcome.outcome, TurnOutcome::Reply(_)));
    }

    // The semaphore admitted the limit's worth at once, and never more.
    assert_eq!(probe.peak.load(Ordering::SeqCst), limit);
}

#[tokio::test(start_paused = true)]
async fn the_scheduler_driver_fires_due_wakeups_on_a_tick() {
    // The background driver fires globally-due wake-ups on a timer, with no session open — the piece
    // deferred from Stage 9 (spec §Scheduled work). Two clocks are in play: `tokio::time::advance`
    // drives the tick (virtual time), while firing reads the `ManualClock`, so we move the manual clock
    // past the occurrence and then advance tokio time to trip a tick.
    let clock = ManualClock::new(test_now());
    let mut store = MemoryStore::new();
    // Watch the log directly, so we observe the driver's firing without opening a session (which would
    // fire via the open-time catch-up and blur which path fired it).
    let events = store.subscribe();
    let server = Server::new(
        Box::new(store),
        Graph::open_in_memory().unwrap(),
        Box::new(clock.clone()),
    );
    // `create_agent` baselines the describer cursor past genesis, so the open-time catch-up does not
    // regenerate the seeded self with this test's scripted occurrence response.
    server.control().create_agent(&seed()).unwrap();

    // Plant a calendared item dated weeks ahead (the turn-end synthesis dates the entry), so it is not
    // yet due when written.
    let plant = ScriptedModel::new([
        run_lua_call(
            r#"memory.get("person/dave@chat"):append("dentist cleaning", { by_agent = true, visibility = "public" })"#,
        ),
        Completion::Reply("noted".to_owned()),
        Completion::Reply(
            serde_json::json!({
                "description": "Dave.",
                "occurrences": [{ "entry": 1, "occurred_at": { "day": "2026-07-01" } }],
            })
            .to_string(),
        ),
    ]);
    server
        .platform()
        .route_message(
            &plant,
            &ConversationLocator::new(TEST_PLATFORM, "leads"),
            &PersonId::new(TEST_PLATFORM, "dave"),
            "remind me about the dentist",
            &[PersonId::new(TEST_PLATFORM, "dave")],
        )
        .await
        .unwrap();

    // Temporal extraction (which schedules the calendared item) runs off the hot path; drive the
    // catch-up so the wake-up is scheduled before the clock moves past it.
    server.describe_catch_up(&plant).await.unwrap();

    // Move the manual clock past the occurrence; the item is now due, but no session opens.
    clock.advance_millis(30 * MILLIS_PER_DAY);

    // Spawn the driver on the shared server with a short tick and a one-shot shutdown.
    let server = Arc::new(server);
    let (stop, shutdown) = tokio::sync::oneshot::channel::<()>();
    let driver = tokio::spawn(
        server
            .clone()
            .run_scheduler(Duration::from_secs(60), async move {
                let _ = shutdown.await;
            }),
    );

    // Trip one tick on the paused tokio clock and let the driver run it, then look for the firing.
    tokio::time::advance(Duration::from_secs(61)).await;
    let mut fired = false;
    for _ in 0..16 {
        tokio::task::yield_now().await;
        while let Ok(event) = events.try_recv() {
            if matches!(event.payload, EventPayload::ScheduledJobFired { .. }) {
                fired = true;
            }
        }
        if fired {
            break;
        }
    }
    assert!(fired, "the driver should fire the due wake-up on a tick");

    // It shuts down cleanly on the signal.
    let _ = stop.send(());
    driver.await.expect("the driver task joins");
}

#[tokio::test(start_paused = true)]
async fn a_wakeup_fired_before_the_idle_close_surfaces_at_the_reopen() {
    // The interleaving under test: the background driver fires a due wake-up while the session is
    // still open, the idle sweep then closes that session, and the next message reopens the
    // conversation. The reopened session's open-time drain must surface the fired item — a fire that
    // precedes the close must not strand the reminder waiting for a further session.
    let clock = ManualClock::new(test_now());
    let mut store = MemoryStore::new();
    let events = store.subscribe();
    let server = Server::new(
        Box::new(store),
        Graph::open_in_memory().unwrap(),
        Box::new(clock.clone()),
    );
    server.control().create_agent(&seed()).unwrap();
    let leads = ConversationLocator::new(TEST_PLATFORM, "leads");

    // Plant a calendared item dated weeks ahead, scheduled by the turn-end synthesis.
    let plant = ScriptedModel::new([
        run_lua_call(
            r#"memory.get("person/dave@chat"):append("dentist cleaning", { by_agent = true, visibility = "public" })"#,
        ),
        Completion::Reply("noted".to_owned()),
        Completion::Reply(
            serde_json::json!({
                "description": "Dave.",
                "occurrences": [{ "entry": 1, "occurred_at": { "day": "2026-07-01" } }],
            })
            .to_string(),
        ),
    ]);
    server
        .platform()
        .route_message(
            &plant,
            &leads,
            &PersonId::new(TEST_PLATFORM, "dave"),
            "remind me about the dentist",
            &[PersonId::new(TEST_PLATFORM, "dave")],
        )
        .await
        .unwrap();
    server.describe_catch_up(&plant).await.unwrap();

    // Move past the occurrence and the idle gap in one jump — the eval's Advance — and fire via the
    // background driver while session 1 is still open (the sweep has not run yet).
    clock.advance_millis(30 * MILLIS_PER_DAY);
    let server = Arc::new(server);
    let (stop, shutdown) = tokio::sync::oneshot::channel::<()>();
    let driver = tokio::spawn(
        server
            .clone()
            .run_scheduler(Duration::from_secs(60), async move {
                let _ = shutdown.await;
            }),
    );
    tokio::time::advance(Duration::from_secs(61)).await;
    let mut fired = false;
    for _ in 0..16 {
        tokio::task::yield_now().await;
        while let Ok(event) = events.try_recv() {
            if matches!(event.payload, EventPayload::ScheduledJobFired { .. }) {
                fired = true;
            }
        }
        if fired {
            break;
        }
    }
    assert!(
        fired,
        "the driver should fire the due wake-up before the close"
    );
    let _ = stop.send(());
    driver.await.expect("the driver task joins");

    // The idle sweep now closes session 1 — after the fire, matching the observed event order.
    let sweep_model = ScriptedModel::new([Completion::Reply("flushed".to_owned())]);
    assert_eq!(server.sweep_idle_sessions(&sweep_model).await.unwrap(), 1);

    // The next message reopens the conversation; the open-time drain must raise the fired item into
    // the session, and the framing must reach the model.
    let reopened = ScriptedModel::new([Completion::Reply("sure".to_owned())]);
    server
        .platform()
        .route_message(
            &reopened,
            &leads,
            &PersonId::new(TEST_PLATFORM, "dave"),
            "what's up",
            &[PersonId::new(TEST_PLATFORM, "dave")],
        )
        .await
        .unwrap();
    assert!(
        reopened
            .recorded_messages()
            .iter()
            .flatten()
            .any(|message| message.content.contains("have come due")),
        "the reopened session's drain should surface the pre-close fire: {:?}",
        reopened.recorded_messages()
    );
}

#[tokio::test]
async fn a_session_is_reused_within_the_idle_gap_and_reopened_after() {
    let (server, clock) = born_agent();
    let model = ScriptedModel::new([
        Completion::Reply("one".to_owned()),
        Completion::Reply("two".to_owned()),
        // The idle reopen flushes the lapsed session's working state before the new one opens (its
        // 4 turns meet flush_min_turns), so a flush turn falls between the second and third messages.
        Completion::Reply("flushed".to_owned()),
        Completion::Reply("three".to_owned()),
    ]);
    let leads = ConversationLocator::new(TEST_PLATFORM, "leads");

    server
        .platform()
        .route_message(
            &model,
            &leads,
            &PersonId::new(TEST_PLATFORM, "dave"),
            "hi",
            &[PersonId::new(TEST_PLATFORM, "dave")],
        )
        .await
        .unwrap();
    // A second message moments later reuses the same session.
    server
        .platform()
        .route_message(
            &model,
            &leads,
            &PersonId::new(TEST_PLATFORM, "dave"),
            "still here",
            &[PersonId::new(TEST_PLATFORM, "dave")],
        )
        .await
        .unwrap();
    assert_eq!(server.control().sessions(&leads).unwrap().len(), 1);

    // After a gap beyond the idle threshold (1800s), the next message reopens a fresh session.
    advance_past_idle_gap(&server, &clock);
    server
        .platform()
        .route_message(
            &model,
            &leads,
            &PersonId::new(TEST_PLATFORM, "dave"),
            "back again",
            &[PersonId::new(TEST_PLATFORM, "dave")],
        )
        .await
        .unwrap();
    assert_eq!(server.control().sessions(&leads).unwrap().len(), 2);
}

#[tokio::test]
async fn the_idle_sweep_closes_and_flushes_a_stale_session() {
    let (server, clock) = born_agent();
    let model = ScriptedModel::new([
        Completion::Reply("one".to_owned()),
        Completion::Reply("two".to_owned()),
        // The past-idle sweep flushes the stale session (its four turns meet flush_min_turns).
        Completion::Reply("flushed".to_owned()),
        Completion::Reply("back".to_owned()),
    ]);
    let leads = ConversationLocator::new(TEST_PLATFORM, "leads");

    // Open a session with enough turns to flush.
    server
        .platform()
        .route_message(
            &model,
            &leads,
            &PersonId::new(TEST_PLATFORM, "dave"),
            "hi",
            &[PersonId::new(TEST_PLATFORM, "dave")],
        )
        .await
        .unwrap();
    server
        .platform()
        .route_message(
            &model,
            &leads,
            &PersonId::new(TEST_PLATFORM, "dave"),
            "still here",
            &[PersonId::new(TEST_PLATFORM, "dave")],
        )
        .await
        .unwrap();
    assert_eq!(server.control().sessions(&leads).unwrap().len(), 1);

    // Within the idle gap, a sweep leaves the session open.
    assert_eq!(server.sweep_idle_sessions(&model).await.unwrap(), 0);
    assert_eq!(server.control().sessions(&leads).unwrap().len(), 1);

    // Past the idle gap, the sweep closes-with-flush it without any message arriving.
    advance_past_idle_gap(&server, &clock);
    assert_eq!(server.sweep_idle_sessions(&model).await.unwrap(), 1);

    // The session is now ended, so the next message opens a fresh one — confirming the close.
    server
        .platform()
        .route_message(
            &model,
            &leads,
            &PersonId::new(TEST_PLATFORM, "dave"),
            "back",
            &[PersonId::new(TEST_PLATFORM, "dave")],
        )
        .await
        .unwrap();
    assert_eq!(server.control().sessions(&leads).unwrap().len(), 2);
}

#[tokio::test]
async fn a_restart_within_the_idle_gap_resumes_the_open_session() {
    let clock = ManualClock::new(test_now());
    let leads = ConversationLocator::new(TEST_PLATFORM, "leads");
    let model = ScriptedModel::new([
        Completion::Reply("one".to_owned()),
        Completion::Reply("two".to_owned()),
    ]);

    // First process: a message opens a session. Its whole log is snapshotted before the instance drops.
    let (opened, log) = {
        let mut server = Server::new(
            Box::new(MemoryStore::new()),
            Graph::open_in_memory().unwrap(),
            Box::new(clock.clone()),
        );
        server.boot().unwrap();
        server.control().create_agent(&seed()).unwrap();
        server
            .platform()
            .route_message(
                &model,
                &leads,
                &PersonId::new(TEST_PLATFORM, "dave"),
                "hi",
                &[PersonId::new(TEST_PLATFORM, "dave")],
            )
            .await
            .unwrap();
        let sessions = server.control().sessions(&leads).unwrap();
        assert_eq!(sessions.len(), 1);
        (sessions[0].id, server.control().events().unwrap())
    }; // the server — and its in-memory session map — drops: a restart

    // Second process: a fresh instance over the *same log* (carried in memory, no temp file), an empty
    // session map, but the log still holds the open session. Within the idle gap, the next message
    // resumes it rather than opening a new one.
    let mut server = Server::new(
        Box::new(MemoryStore::from_events(log)),
        Graph::open_in_memory().unwrap(),
        Box::new(clock.clone()),
    );
    server.boot().unwrap();
    server
        .platform()
        .route_message(
            &model,
            &leads,
            &PersonId::new(TEST_PLATFORM, "dave"),
            "still here",
            &[PersonId::new(TEST_PLATFORM, "dave")],
        )
        .await
        .unwrap();
    let sessions = server.control().sessions(&leads).unwrap();
    assert_eq!(
        sessions.len(),
        1,
        "resumed the open session; no new session opened"
    );
    assert_eq!(sessions[0].id, opened, "the resumed session keeps its id");
}

#[tokio::test]
async fn a_reopen_after_a_restart_reconstructs_the_prior_tail_from_the_log() {
    // Issue #86: the carryover is reconstructed from the event log at reopen, not cached in runtime
    // state, so a session's tail survives a restart between the close and the reopen — the case an
    // in-memory stash always lost. Process 1 idle-sweeps a session closed (its `SessionEnded` lands in
    // the log with no successor), then only its *log* is carried into a fresh process 2 (its session map
    // and any runtime state reset), which must still carry that session's messages into the reopen.
    let clock = ManualClock::new(test_now());
    let leads = ConversationLocator::new(TEST_PLATFORM, "leads");
    let dave = PersonId::new(TEST_PLATFORM, "dave");

    // Process 1: a message opens a session; past the idle gap the sweep closes it with no message
    // arriving, so the log holds a `SessionEnded` and no later session. Its whole log is snapshotted;
    // the instance (its session map, and under the old design any staged carryover) is then dropped.
    let log = {
        let model =
            ScriptedModel::new([Completion::Reply("the migration ships Friday".to_owned())]);
        let mut server = Server::new(
            Box::new(MemoryStore::new()),
            Graph::open_in_memory().unwrap(),
            Box::new(clock.clone()),
        );
        server.boot().unwrap();
        server.control().create_agent(&seed()).unwrap();
        server
            .platform()
            .route_message(
                &model,
                &leads,
                &dave,
                "when does the migration ship?",
                std::slice::from_ref(&dave),
            )
            .await
            .unwrap();
        advance_past_idle_gap(&server, &clock);
        assert_eq!(
            server.sweep_idle_sessions(&model).await.unwrap(),
            1,
            "the idle sweep closes the session past the gap"
        );
        server.control().events().unwrap()
    }; // process 1 (and its session map) drops — only the log survives, as across a restart

    // Process 2: a fresh instance over the *same log* (carried in memory, no temp file), empty session
    // map, no cached carryover. The next message reopens; the swept session's tail must be reconstructed
    // from the log.
    let model = ScriptedModel::new([Completion::Reply("still Friday".to_owned())]);
    let mut server = Server::new(
        Box::new(MemoryStore::from_events(log)),
        Graph::open_in_memory().unwrap(),
        Box::new(clock.clone()),
    );
    server.boot().unwrap();
    server
        .platform()
        .route_message(
            &model,
            &leads,
            &dave,
            "remind me — when does it ship?",
            std::slice::from_ref(&dave),
        )
        .await
        .unwrap();

    // The reopened session's prompt carries the pre-restart session's message, reconstructed from the
    // log rather than lost with process 1.
    let prompt: String = model
        .recorded_messages()
        .iter()
        .flatten()
        .map(|message| message.content.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        prompt.contains("when does the migration ship?"),
        "the reopened prompt must carry the pre-restart tail, reconstructed from the log; got:\n{prompt}"
    );
}
