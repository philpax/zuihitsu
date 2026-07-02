//! Server tests via the in-process control client: creating an agent, inspecting it, idempotent
//! re-creation, and boot reconciling a fresh graph from a persisted log (spec §Clients, §Storage).

mod common;

use std::time::Duration;
use zuihitsu::{
    Completion, ConcurrencySettings, ConversationLocator, Embedder, FakeEmbedder, GenerateRequest,
    GenerateResponse, Graph, InMemoryVectorIndex, ManualClock, MemoryId, MemoryName, MemoryStore,
    ModelClient, ModelError, Namespace, ScriptedModel, SeedSelf, Server, SqliteStore, Store,
    ToolCall, TurnOutcome, TurnRole, Usage, VectorIndex,
    event::{EventPayload, MergeProposalSource},
    genesis::{GenesisStatus, Rollout},
    time::MILLIS_PER_DAY,
};

use common::time::TEST_NOW;

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

fn seed() -> SeedSelf {
    SeedSelf {
        agent_name: "Kestrel".to_owned(),
        persona: "A thoughtful, discreet companion with a long memory.".to_owned(),
        seed_entries: vec!["I keep what people tell me in confidence.".to_owned()],
    }
}

fn clock() -> Box<ManualClock> {
    Box::new(ManualClock::new(TEST_NOW))
}

#[test]
fn control_creates_and_inspects_an_agent() {
    let mut server = Server::in_memory(clock()).unwrap();
    assert_eq!(server.boot().unwrap(), GenesisStatus::Empty);

    let outcome = server.control().create_agent(&seed()).unwrap();
    assert!(matches!(outcome, Rollout::Created { .. }));

    assert_eq!(
        server.control().genesis_status().unwrap(),
        GenesisStatus::Complete
    );
    assert_eq!(
        server
            .control()
            .memory("self")
            .unwrap()
            .unwrap()
            .name
            .as_str(),
        "self"
    );
    assert_eq!(server.control().settings().unwrap().turn.max_steps, 12);
    assert!(server.control().memory("person/nobody").unwrap().is_none());

    // Creating again is a no-op on a born agent.
    assert_eq!(
        server.control().create_agent(&seed()).unwrap(),
        Rollout::AlreadyComplete
    );
}

#[test]
fn boot_reconciles_a_fresh_graph_from_a_persisted_log() {
    let path =
        std::env::temp_dir().join(format!("zuihitsu-server-{}.sqlite", MemoryId::generate().0));

    {
        let store = SqliteStore::open(&path).unwrap();
        let mut server = Server::new(Box::new(store), Graph::open_in_memory().unwrap(), clock());
        server.boot().unwrap();
        server.control().create_agent(&seed()).unwrap();
    } // the store (and its log lock) drop here

    {
        // Reopen the persisted log with a brand-new, empty graph: boot must catch it up to
        // log-head before the agent is inspectable.
        let store = SqliteStore::open(&path).unwrap();
        let mut server = Server::new(Box::new(store), Graph::open_in_memory().unwrap(), clock());
        assert_eq!(server.boot().unwrap(), GenesisStatus::Complete);
        assert!(server.control().memory("self").unwrap().is_some());
    }

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
    let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
}

#[test]
fn a_server_snapshot_captures_the_graph_at_its_head() {
    let mut server = Server::in_memory(clock()).unwrap();
    server.boot().unwrap();
    server.control().create_agent(&seed()).unwrap();

    let dir = std::env::temp_dir().join(format!("zuihitsu-snap-{}", MemoryId::generate().0));
    let path = server
        .snapshot(&dir)
        .unwrap()
        .expect("a first snapshot is written");

    // The snapshot is a self-describing graph at a real (non-zero) head, with the born agent's state.
    assert!(zuihitsu::snapshot::read_graph_head(&path).unwrap().0 > 0);
    let restored = Graph::open(&path).unwrap();
    assert!(
        restored
            .memory_by_name(MemoryName::self_handle())
            .unwrap()
            .is_some()
    );

    // A second snapshot with no events since is a no-op (already checkpointed at this head).
    assert!(server.snapshot(&dir).unwrap().is_none());

    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn the_indexer_catches_the_vector_index_up_to_the_log() {
    let embedder: std::sync::Arc<dyn Embedder> = std::sync::Arc::new(FakeEmbedder::new(16));
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
fn born_agent() -> (Server, ManualClock) {
    let clock = ManualClock::new(TEST_NOW);
    let server = Server::new(
        Box::new(MemoryStore::new()),
        Graph::open_in_memory().unwrap(),
        Box::new(clock.clone()),
    );
    // `create_agent` baselines the describer cursor past genesis, so a synchronous catch-up never tries
    // to regenerate the seeded `self` with a scripted response meant for the test's own writes.
    server.control().create_agent(&seed()).unwrap();
    (server, clock)
}

#[tokio::test]
async fn route_message_opens_a_session_and_runs_a_turn() {
    let (server, _clock) = born_agent();
    let model = ScriptedModel::new([Completion::Reply("Hi, Dave.".to_owned())]);
    let leads = ConversationLocator::new("discord", "leads");

    let outcome = server
        .platform()
        .route_message(&model, &leads, "dave", "hello there", &["dave"])
        .await
        .unwrap();
    assert_eq!(outcome, TurnOutcome::Reply("Hi, Dave.".to_owned()));

    // First contact minted the room's context and the sender's stub.
    assert!(
        server
            .control()
            .memory("context/discord:leads")
            .unwrap()
            .is_some()
    );
    assert!(server.control().memory("person/dave").unwrap().is_some());

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

    async fn generate(&self, _request: &GenerateRequest) -> Result<GenerateResponse, ModelError> {
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
            let room = ConversationLocator::new("discord", format!("room-{i}"));
            let sender = format!("user-{i}");
            server
                .platform()
                .route_message(probe.as_ref(), &room, &sender, "ping", &[sender.as_str()])
                .await
        }));
    }
    for task in tasks {
        let outcome = task.await.expect("the turn task joins").expect("turn runs");
        assert!(matches!(outcome, TurnOutcome::Reply(_)));
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
    let clock = ManualClock::new(TEST_NOW);
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
            r#"memory.get("person/dave"):append("dentist cleaning", { by_agent = true, visibility = "public" })"#,
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
            &ConversationLocator::new("discord", "leads"),
            "dave",
            "remind me about the dentist",
            &["dave"],
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
    let leads = ConversationLocator::new("discord", "leads");

    server
        .platform()
        .route_message(&model, &leads, "dave", "hi", &["dave"])
        .await
        .unwrap();
    // A second message moments later reuses the same session.
    server
        .platform()
        .route_message(&model, &leads, "dave", "still here", &["dave"])
        .await
        .unwrap();
    assert_eq!(server.control().sessions(&leads).unwrap().len(), 1);

    // After a gap beyond the idle threshold (1800s), the next message reopens a fresh session.
    clock.advance_millis(1_801 * 1_000);
    server
        .platform()
        .route_message(&model, &leads, "dave", "back again", &["dave"])
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
    let leads = ConversationLocator::new("discord", "leads");

    // Open a session with enough turns to flush.
    server
        .platform()
        .route_message(&model, &leads, "dave", "hi", &["dave"])
        .await
        .unwrap();
    server
        .platform()
        .route_message(&model, &leads, "dave", "still here", &["dave"])
        .await
        .unwrap();
    assert_eq!(server.control().sessions(&leads).unwrap().len(), 1);

    // Within the idle gap, a sweep leaves the session open.
    assert_eq!(server.sweep_idle_sessions(&model).await.unwrap(), 0);
    assert_eq!(server.control().sessions(&leads).unwrap().len(), 1);

    // Past the idle gap, the sweep closes-with-flush it without any message arriving.
    clock.advance_millis(1_801 * 1_000);
    assert_eq!(server.sweep_idle_sessions(&model).await.unwrap(), 1);

    // The session is now ended, so the next message opens a fresh one — confirming the close.
    server
        .platform()
        .route_message(&model, &leads, "dave", "back", &["dave"])
        .await
        .unwrap();
    assert_eq!(server.control().sessions(&leads).unwrap().len(), 2);
}

#[tokio::test]
async fn a_restart_within_the_idle_gap_resumes_the_open_session() {
    let path =
        std::env::temp_dir().join(format!("zuihitsu-resume-{}.sqlite", MemoryId::generate().0));
    let clock = ManualClock::new(TEST_NOW);
    let leads = ConversationLocator::new("discord", "leads");
    let model = ScriptedModel::new([
        Completion::Reply("one".to_owned()),
        Completion::Reply("two".to_owned()),
    ]);

    // First process: a message opens a session.
    let opened = {
        let mut server = Server::new(
            Box::new(SqliteStore::open(&path).unwrap()),
            Graph::open_in_memory().unwrap(),
            Box::new(clock.clone()),
        );
        server.boot().unwrap();
        server.control().create_agent(&seed()).unwrap();
        server
            .platform()
            .route_message(&model, &leads, "dave", "hi", &["dave"])
            .await
            .unwrap();
        let sessions = server.control().sessions(&leads).unwrap();
        assert_eq!(sessions.len(), 1);
        sessions[0].id
    }; // the server — and its in-memory session map — drops: a restart

    // Second process: an empty session map, but the log still holds the open session. Within the idle
    // gap, the next message resumes it rather than opening a new one.
    let mut server = Server::new(
        Box::new(SqliteStore::open(&path).unwrap()),
        Graph::open_in_memory().unwrap(),
        Box::new(clock.clone()),
    );
    server.boot().unwrap();
    server
        .platform()
        .route_message(&model, &leads, "dave", "still here", &["dave"])
        .await
        .unwrap();
    let sessions = server.control().sessions(&leads).unwrap();
    assert_eq!(
        sessions.len(),
        1,
        "resumed the open session; no new session opened"
    );
    assert_eq!(sessions[0].id, opened, "the resumed session keeps its id");

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
    let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
}

#[tokio::test]
async fn a_restart_past_the_idle_gap_flushes_and_reopens() {
    let path =
        std::env::temp_dir().join(format!("zuihitsu-reopen-{}.sqlite", MemoryId::generate().0));
    let clock = ManualClock::new(TEST_NOW);
    let leads = ConversationLocator::new("discord", "leads");
    let model = ScriptedModel::new([
        Completion::Reply("one".to_owned()),
        Completion::Reply("two".to_owned()),
        // The recovery close flushes the lapsed session — its four turns meet flush_min_turns.
        Completion::Reply("flushed".to_owned()),
        Completion::Reply("three".to_owned()),
    ]);

    // First process: two messages — four turns, enough to trigger the flush on close.
    {
        let mut server = Server::new(
            Box::new(SqliteStore::open(&path).unwrap()),
            Graph::open_in_memory().unwrap(),
            Box::new(clock.clone()),
        );
        server.boot().unwrap();
        server.control().create_agent(&seed()).unwrap();
        server
            .platform()
            .route_message(&model, &leads, "dave", "hi", &["dave"])
            .await
            .unwrap();
        server
            .platform()
            .route_message(&model, &leads, "dave", "still here", &["dave"])
            .await
            .unwrap();
        assert_eq!(server.control().sessions(&leads).unwrap().len(), 1);
    } // restart

    // Second process: past the idle gap, the next message closes the recovered session (flushing its
    // working state) and opens a fresh one.
    let mut server = Server::new(
        Box::new(SqliteStore::open(&path).unwrap()),
        Graph::open_in_memory().unwrap(),
        Box::new(clock.clone()),
    );
    server.boot().unwrap();
    clock.advance_millis(1_801 * 1_000);
    server
        .platform()
        .route_message(&model, &leads, "dave", "back again", &["dave"])
        .await
        .unwrap();
    assert_eq!(
        server.control().sessions(&leads).unwrap().len(),
        2,
        "the stale recovered session closed and a fresh one opened"
    );

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
    let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
}

#[tokio::test]
async fn note_join_records_the_arriving_participant_on_the_session() {
    let (server, _clock) = born_agent();
    let model = ScriptedModel::new([Completion::Reply("hi".to_owned())]);
    let leads = ConversationLocator::new("discord", "leads");

    // Open a session with Dave present.
    server
        .platform()
        .route_message(&model, &leads, "dave", "hi", &["dave"])
        .await
        .unwrap();
    let dave = server.control().memory("person/dave").unwrap().unwrap().id;

    // Erin joins mid-session via the explicit endpoint path — with no model configured, so the
    // join-brief composes off the current prose rather than failing.
    server
        .platform()
        .note_join(None, &leads, "erin")
        .await
        .unwrap();
    let erin = server.control().memory("person/erin").unwrap().unwrap().id;

    let sessions = server.control().sessions(&leads).unwrap();
    assert_eq!(sessions.len(), 1);
    let participants = &sessions[0].participants;
    assert!(participants.contains(&dave));
    assert!(participants.contains(&erin));

    // The endpoint shares the per-message sync's code path: the same join-brief system turn lands.
    let events = server.control().events().unwrap();
    assert!(
        events.iter().any(|event| matches!(
            &event.payload,
            EventPayload::ConversationTurn {
                role: TurnRole::System,
                participant: Some(participant),
                ..
            } if *participant == erin
        )),
        "note_join injects the same join-brief as the per-message sync"
    );
}

#[tokio::test]
async fn a_newcomers_first_mid_session_message_injects_a_join_brief_before_their_turn() {
    let (server, _clock) = born_agent();
    let leads = ConversationLocator::new("discord", "leads");
    let model = ScriptedModel::new([
        Completion::Reply("hi dave".to_owned()),
        Completion::Reply("hi erin".to_owned()),
    ]);

    // Dave opens the session alone; Erin's first message arrives mid-session, with no explicit
    // /platform/join posted — the message itself is the join signal.
    server
        .platform()
        .route_message(&model, &leads, "dave", "morning", &["dave"])
        .await
        .unwrap();
    server
        .platform()
        .route_message(&model, &leads, "erin", "hey, joining in", &["dave", "erin"])
        .await
        .unwrap();

    let erin = server.control().memory("person/erin").unwrap().unwrap().id;
    let events = server.control().events().unwrap();

    // Exactly one join was recorded for the newcomer.
    let joins = events
        .iter()
        .filter(|event| {
            matches!(
                &event.payload,
                EventPayload::ParticipantJoined { participant, .. } if *participant == erin
            )
        })
        .count();
    assert_eq!(joins, 1, "one ParticipantJoined for the newcomer");

    // The injected join-brief — a system turn about Erin — precedes her inbound turn in the log,
    // and its text reflects her memory.
    let (brief_seq, brief_text) = events
        .iter()
        .find_map(|event| match &event.payload {
            EventPayload::ConversationTurn {
                role: TurnRole::System,
                participant: Some(participant),
                text,
                ..
            } if *participant == erin => Some((event.seq, text.clone())),
            _ => None,
        })
        .expect("the join injected a system join-brief turn");
    let inbound_seq = events
        .iter()
        .find_map(|event| match &event.payload {
            EventPayload::ConversationTurn {
                role: TurnRole::Participant,
                participant: Some(participant),
                ..
            } if *participant == erin => Some(event.seq),
            _ => None,
        })
        .expect("Erin's inbound turn is in the log");
    assert!(
        brief_seq.0 < inbound_seq.0,
        "the join-brief precedes the joiner's inbound turn"
    );
    assert!(
        brief_text.contains("person/erin"),
        "the join-brief reflects the joiner's memory: {brief_text}"
    );

    // The join reused the live session, whose participants now include both.
    let dave = server.control().memory("person/dave").unwrap().unwrap().id;
    let sessions = server.control().sessions(&leads).unwrap();
    assert_eq!(sessions.len(), 1, "the join reused the live session");
    assert!(sessions[0].participants.contains(&dave));
    assert!(sessions[0].participants.contains(&erin));
}

#[tokio::test]
async fn a_participant_merely_present_on_a_message_is_joined_too() {
    let (server, _clock) = born_agent();
    let leads = ConversationLocator::new("discord", "leads");
    let model = ScriptedModel::new([
        Completion::Reply("hi dave".to_owned()),
        Completion::Reply("noted".to_owned()),
    ]);

    server
        .platform()
        .route_message(&model, &leads, "dave", "morning", &["dave"])
        .await
        .unwrap();
    // Erin never speaks — she only appears in Dave's present list — yet the presence sync joins her.
    server
        .platform()
        .route_message(
            &model,
            &leads,
            "dave",
            "erin just walked in",
            &["dave", "erin"],
        )
        .await
        .unwrap();

    let erin = server.control().memory("person/erin").unwrap().unwrap().id;
    let events = server.control().events().unwrap();
    assert!(
        events.iter().any(|event| matches!(
            &event.payload,
            EventPayload::ParticipantJoined { participant, .. } if *participant == erin
        )),
        "presence alone records the join"
    );
    let sessions = server.control().sessions(&leads).unwrap();
    assert!(sessions[0].participants.contains(&erin));
}

#[tokio::test]
async fn repeat_messages_from_the_same_joiner_do_not_re_join() {
    let (server, _clock) = born_agent();
    let leads = ConversationLocator::new("discord", "leads");
    let model = ScriptedModel::new([
        Completion::Reply("hi dave".to_owned()),
        Completion::Reply("hi erin".to_owned()),
        Completion::Reply("still here".to_owned()),
    ]);

    server
        .platform()
        .route_message(&model, &leads, "dave", "morning", &["dave"])
        .await
        .unwrap();
    server
        .platform()
        .route_message(&model, &leads, "erin", "hi", &["dave", "erin"])
        .await
        .unwrap();
    server
        .platform()
        .route_message(&model, &leads, "erin", "one more thing", &["dave", "erin"])
        .await
        .unwrap();

    let erin = server.control().memory("person/erin").unwrap().unwrap().id;
    let events = server.control().events().unwrap();
    let joins = events
        .iter()
        .filter(|event| {
            matches!(
                &event.payload,
                EventPayload::ParticipantJoined { participant, .. } if *participant == erin
            )
        })
        .count();
    assert_eq!(joins, 1, "a member's later messages do not re-join");
}

#[tokio::test]
async fn a_due_wakeup_is_drained_into_the_next_eligible_session() {
    let (server, clock) = born_agent();
    let leads = ConversationLocator::new("discord", "leads");

    // Turn 1: the agent records a note on Dave's memory and the turn-end synthesis dates it to
    // 2026-07-01 — a calendared item scheduled weeks after the present TEST_NOW.
    let plant = ScriptedModel::new([
        run_lua_call(
            r#"memory.get("person/dave"):append("dentist cleaning", { by_agent = true, visibility = "public" })"#,
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
            "dave",
            "remind me about the dentist",
            &["dave"],
        )
        .await
        .unwrap();

    // Temporal extraction runs off the hot path; drive the catch-up so the calendared item is
    // scheduled before the clock advances past it.
    server.describe_catch_up(&plant).await.unwrap();

    // Advance past the occurrence and the idle gap, so the next message opens a fresh session.
    clock.advance_millis(30 * 86_400_000_i64);

    // Turn 2: opening this session fires the now-due wake-up and drains it as a system turn the agent
    // sees in its buffer.
    let drained = ScriptedModel::new([Completion::Reply("sure".to_owned())]);
    server
        .platform()
        .route_message(&drained, &leads, "dave", "what's up", &["dave"])
        .await
        .unwrap();
    assert!(
        drained
            .recorded_messages()
            .iter()
            .flatten()
            .any(|message| message.content.contains("have come due")),
        "the drain should reach the model: {:?}",
        drained.recorded_messages()
    );

    // A later session: the item is surfaced, so it is never raised a second time.
    clock.advance_millis(2 * 86_400_000_i64);
    let quiet = ScriptedModel::new([Completion::Reply("ok".to_owned())]);
    server
        .platform()
        .route_message(&quiet, &leads, "dave", "still here", &["dave"])
        .await
        .unwrap();
    assert!(
        quiet
            .recorded_messages()
            .iter()
            .flatten()
            .all(|message| !message.content.contains("have come due")),
        "a surfaced item must not be raised again",
    );
}

#[tokio::test]
async fn a_token_budget_crossing_forces_a_re_segment_within_the_idle_gap() {
    let (server, _clock) = born_agent();
    // A tight token budget, so a single reported usage crosses it.
    let mut settings = server.control().settings().unwrap();
    settings.compaction.token_budget = 100;
    server.control().set_settings(settings).unwrap();

    let leads = ConversationLocator::new("discord", "leads");
    // Turn 1 reports usage over the budget; turn 2 is well under. Both arrive within the idle gap, so
    // only the token trigger — not idle — can force a second session.
    let model = ScriptedModel::with_usage([
        (Completion::Reply("one".to_owned()), 200),
        (Completion::Reply("two".to_owned()), 10),
    ]);

    server
        .platform()
        .route_message(&model, &leads, "dave", "hi", &["dave"])
        .await
        .unwrap();
    assert_eq!(server.control().sessions(&leads).unwrap().len(), 1);

    server
        .platform()
        .route_message(&model, &leads, "dave", "still here", &["dave"])
        .await
        .unwrap();
    let sessions = server.control().sessions(&leads).unwrap();
    assert_eq!(sessions.len(), 2);
    // The first session opened fresh; the re-segmented one carries a tail and re-freezes a brief.
    assert!(sessions[0].seeded_from_turn.is_none());
    assert!(sessions[1].seeded_from_turn.is_some());
    assert!(!sessions[1].brief.is_empty());
}

#[tokio::test]
async fn the_live_buffer_is_replayed_to_the_model_on_later_turns() {
    let (server, _clock) = born_agent();
    let leads = ConversationLocator::new("discord", "leads");
    let model = ScriptedModel::new([
        Completion::Reply("first reply".to_owned()),
        Completion::Reply("second reply".to_owned()),
    ]);

    server
        .platform()
        .route_message(&model, &leads, "dave", "hello there", &["dave"])
        .await
        .unwrap();
    server
        .platform()
        .route_message(&model, &leads, "dave", "and again", &["dave"])
        .await
        .unwrap();

    let seen = model.recorded_messages();
    assert_eq!(seen.len(), 2);
    // Turn 1's prompt is just the inbound message, stamped with who spoke and the time it was recorded
    // (TEST_NOW; the clock does not advance in this test). The agent reads it, so it carries a
    // speaker-and-time prefix that lets it attribute the turn in a multi-party room.
    let turn1: Vec<&str> = seen[0]
        .iter()
        .map(|message| message.content.as_str())
        .collect();
    assert_eq!(turn1, vec!["[Mon 2026-06-08 00:00 UTC] dave: hello there"]);
    // Turn 2 replays the live buffer — turn 1's participant and agent turns — then the new inbound.
    // The participant turns it reads are speaker-and-time-stamped; the agent's own reply is left
    // unstamped (its `assistant` role already identifies it).
    let turn2: Vec<&str> = seen[1]
        .iter()
        .map(|message| message.content.as_str())
        .collect();
    assert_eq!(
        turn2,
        vec![
            "[Mon 2026-06-08 00:00 UTC] dave: hello there",
            "first reply",
            "[Mon 2026-06-08 00:00 UTC] dave: and again",
        ]
    );
}

#[tokio::test]
async fn each_turn_carries_its_own_recorded_time() {
    let (server, clock) = born_agent();
    let leads = ConversationLocator::new("discord", "leads");
    let model = ScriptedModel::new([
        Completion::Reply("morning".to_owned()),
        Completion::Reply("still morning".to_owned()),
    ]);

    server
        .platform()
        .route_message(&model, &leads, "dave", "first message", &["dave"])
        .await
        .unwrap();
    // Ten minutes later, within the idle gap, so the same session continues and the buffer replays.
    clock.advance_millis(600 * 1_000);
    server
        .platform()
        .route_message(&model, &leads, "dave", "second message", &["dave"])
        .await
        .unwrap();

    // The second turn's prompt shows the first message frozen at its original time and the new inbound
    // at the advanced time — "now" tracks the clock without the historical stamp drifting.
    let seen = model.recorded_messages();
    let turn2: Vec<&str> = seen
        .last()
        .unwrap()
        .iter()
        .map(|message| message.content.as_str())
        .collect();
    assert_eq!(
        turn2,
        vec![
            "[Mon 2026-06-08 00:00 UTC] dave: first message",
            "morning",
            "[Mon 2026-06-08 00:10 UTC] dave: second message",
        ]
    );
}

fn run_lua_call(script: &str) -> Completion {
    Completion::ToolCalls(vec![ToolCall {
        id: "lua".to_owned(),
        name: "run_lua".to_owned(),
        arguments: serde_json::json!({ "script": script }).to_string(),
    }])
}

fn describe_call(description: &str) -> Completion {
    // The turn-end synthesis is a `response_format`-constrained reply carrying the description as JSON
    // (these scenarios plant no temporal phrases, so no occurrences).
    Completion::Reply(
        serde_json::json!({ "description": description, "occurrences": [] }).to_string(),
    )
}

#[tokio::test]
async fn a_substantive_session_flushes_to_memory_before_the_cut() {
    let (server, _clock) = born_agent();
    let mut settings = server.control().settings().unwrap();
    settings.compaction.token_budget = 100;
    // The default flush gate is four turns; the two exchanges below reach it.
    server.control().set_settings(settings).unwrap();

    let leads = ConversationLocator::new("discord", "leads");
    let model = ScriptedModel::with_usage([
        // Two ordinary exchanges build the session to four turns; the second crosses the budget.
        (Completion::Reply("ok one".to_owned()), 10),
        (Completion::Reply("ok two".to_owned()), 200),
        // The pre-compaction flush writes durable state, then confirms.
        (
            run_lua_call(r#"memory.create("topic/plan", "Decided to ship on Friday")"#),
            0,
        ),
        (Completion::Reply("flushed".to_owned()), 0),
        // Genesis registered the description-regen template, so the flushed memory is regenerated.
        (describe_call("The team's plan to ship on Friday."), 0),
    ]);

    server
        .platform()
        .route_message(&model, &leads, "dave", "morning", &["dave"])
        .await
        .unwrap();
    server
        .platform()
        .route_message(&model, &leads, "dave", "any updates", &["dave"])
        .await
        .unwrap();
    // The flush's writes are described off the hot path; drive the catch-up to regenerate them.
    server.describe_catch_up(&model).await.unwrap();

    // Only the flush calls run_lua here, so the memory's presence is the flush's signature — it wrote
    // the working state to memory, its description regenerated by the off-hot-path catch-up.
    let plan = server.control().memory("topic/plan").unwrap();
    assert!(plan.is_some());
    assert_eq!(
        plan.unwrap().description,
        "The team's plan to ship on Friday."
    );
}

#[tokio::test]
async fn a_low_activity_session_skips_the_flush() {
    let (server, _clock) = born_agent();
    let mut settings = server.control().settings().unwrap();
    settings.compaction.token_budget = 100;
    server.control().set_settings(settings).unwrap();

    let leads = ConversationLocator::new("discord", "leads");
    // A single exchange (two turns) crosses the budget — below the four-turn gate. Only the turn's
    // own response is scripted: were the flush to run, it would call the model again and exhaust the
    // queue, erroring. The route succeeding is what proves the flush was skipped.
    let model =
        ScriptedModel::with_usage([(Completion::Reply("a giant paste, noted".to_owned()), 500)]);

    server
        .platform()
        .route_message(&model, &leads, "dave", "<a huge paste>", &["dave"])
        .await
        .unwrap();

    // The session ended (a re-segment is staged) without a flush turn having run.
    assert_eq!(server.control().sessions(&leads).unwrap().len(), 1);
}

#[tokio::test]
async fn context_current_resolves_during_a_routed_turn() {
    let (server, _clock) = born_agent();
    let leads = ConversationLocator::new("discord", "leads");
    // The agent appends to the current context. If context.current() returned nil in the routed path
    // (as a real-model run's stray `Context: nil` print suggested), this would error on nil:append
    // and commit nothing.
    let model = ScriptedModel::new([
        run_lua_call(r#"context.current():append("a note in the room", { by_agent = true })"#),
        Completion::Reply("noted".to_owned()),
    ]);
    server
        .platform()
        .route_message(&model, &leads, "dave", "hi", &["dave"])
        .await
        .unwrap();
    // The context memory received the entry — context.current() resolved through route_message.
    let entries = server.control().entries("context/discord:leads").unwrap();
    assert!(
        entries
            .iter()
            .any(|entry| entry.text == "a note in the room"),
        "context entries: {entries:?}"
    );
}

#[tokio::test]
async fn the_working_set_carries_into_the_next_session_brief() {
    let (server, _clock) = born_agent();
    let mut settings = server.control().settings().unwrap();
    settings.compaction.token_budget = 100;
    server.control().set_settings(settings).unwrap();

    let leads = ConversationLocator::new("discord", "leads");
    let model = ScriptedModel::with_usage([
        // Turn 1 touches a memory, then crosses the budget (two turns — below the flush gate).
        (
            run_lua_call(r#"memory.create("topic/roadmap", "Plan the Q3 work")"#),
            10,
        ),
        (Completion::Reply("on it".to_owned()), 200),
        // Regeneration of the touched memory's description.
        (describe_call("The team's Q3 roadmap."), 0),
        // Turn 2 opens the re-segmented session; its frozen brief is what we inspect.
        (Completion::Reply("hello again".to_owned()), 0),
    ]);

    server
        .platform()
        .route_message(&model, &leads, "dave", "let's plan", &["dave"])
        .await
        .unwrap();
    server
        .platform()
        .route_message(&model, &leads, "dave", "back", &["dave"])
        .await
        .unwrap();

    let sessions = server.control().sessions(&leads).unwrap();
    assert_eq!(sessions.len(), 2);
    // The re-segmented session's brief re-surfaces the touched memory as an active thread.
    let brief = &sessions[1].brief;
    assert!(brief.contains("# Active threads"), "brief was: {brief}");
    assert!(brief.contains("topic/roadmap"), "brief was: {brief}");
}

#[tokio::test]
async fn the_compaction_working_set_is_the_touched_set_only() {
    // The `_session_carryover` link source is retired (issue #21): the compacted session's working set
    // is purely its touched memories. A thread the agent flagged for carryover in an earlier session
    // but never touched in the session that compacts does not carry, since no agent-managed graph flag
    // feeds the working set anymore — only the platform-derived touched set does.
    let (server, clock) = born_agent();
    let mut settings = server.control().settings().unwrap();
    settings.compaction.token_budget = 100;
    server.control().set_settings(settings).unwrap();

    let leads = ConversationLocator::new("discord", "leads");
    let model = ScriptedModel::with_usage([
        // Session 1: create a thread and (as an old agent might) flag it — an ordinary turn, under budget.
        // The link still materializes; it simply no longer feeds the carryover working set.
        (
            run_lua_call(
                r#"local t = memory.create("topic/migration", "Plan the DB migration"); t:link("_session_carryover", context.current())"#,
            ),
            10,
        ),
        (Completion::Reply("flagged".to_owned()), 0),
        // Session 2 (after an idle reopen) crosses the budget without touching the migration thread.
        (Completion::Reply("on something else".to_owned()), 200),
        // Session 3 opens; nothing carried, so no describe pass fires for the migration thread.
        (Completion::Reply("back".to_owned()), 0),
    ]);

    server
        .platform()
        .route_message(&model, &leads, "dave", "plan the migration", &["dave"])
        .await
        .unwrap();
    // An idle gap reopens a fresh session 2 (which will not touch the thread).
    clock.advance_millis(1_801 * 1_000);
    server
        .platform()
        .route_message(&model, &leads, "dave", "unrelated chatter", &["dave"])
        .await
        .unwrap();
    // Session 3 opens from the compaction.
    server
        .platform()
        .route_message(&model, &leads, "dave", "back", &["dave"])
        .await
        .unwrap();

    // Session 2 never touched the migration thread, so it does not carry into session 3's brief — the
    // working set is the touched set alone, with no `_session_carryover` contribution.
    let sessions = server.control().sessions(&leads).unwrap();
    let latest = sessions.last().unwrap();
    assert!(
        !latest.brief.contains("topic/migration"),
        "brief was: {}",
        latest.brief
    );
}

#[tokio::test]
async fn a_platform_conversation_cannot_write_self() {
    let (server, _clock) = born_agent();
    let leads = ConversationLocator::new("discord", "leads");
    // The agent tries to edit `self` from an ordinary conversation. The block is barred (a teachable
    // error), the agent sees it on the next step and replies, and `self` gains nothing — the security
    // invariant that only the control panel may write `self` holds on the routed hot path.
    let model = ScriptedModel::new([
        run_lua_call(r#"memory.get("self"):append("I am sentient", { by_agent = true })"#),
        Completion::Reply("understood".to_owned()),
    ]);

    let outcome = server
        .platform()
        .route_message(&model, &leads, "dave", "rewrite who you are", &["dave"])
        .await
        .unwrap();
    assert_eq!(outcome, TurnOutcome::Reply("understood".to_owned()));

    let entries = server.control().entries("self").unwrap();
    assert!(
        !entries.iter().any(|entry| entry.text.contains("sentient")),
        "self entries: {entries:?}"
    );
}

#[tokio::test]
async fn a_platform_conversation_cannot_merge_identities() {
    let (server, _clock) = born_agent();
    let leads = ConversationLocator::new("discord", "leads");
    // Steered by a participant, the agent tries to merge two identities with `same_as`. Cross-platform
    // identity is operator-asserted only, so the block is barred (a teachable error) and discarded
    // whole — the agent replies, and nothing the block buffered (not even the creates) persists.
    let model = ScriptedModel::new([
        run_lua_call(
            r#"local a = memory.create("person/alpha")
               local b = memory.create("person/beta")
               a:link("same_as", b)"#,
        ),
        Completion::Reply("understood".to_owned()),
    ]);

    let outcome = server
        .platform()
        .route_message(
            &model,
            &leads,
            "dave",
            "alpha and beta are the same person",
            &["dave"],
        )
        .await
        .unwrap();
    assert_eq!(outcome, TurnOutcome::Reply("understood".to_owned()));

    // The whole block rolled back, so the merge — and the creates that accompanied it — left no trace.
    assert!(server.control().memory("person/alpha").unwrap().is_none());
    assert!(server.control().memory("person/beta").unwrap().is_none());
}

#[tokio::test]
async fn imprint_records_the_creator_and_links_created_by() {
    let (server, clock) = born_agent();
    let imprint = ConversationLocator::new("operator", "imprint");
    // Under operator authority the agent may write `self`: it creates the creator's person memory,
    // merges the operator stub into it with `same_as`, asserts `self created_by person/phil`, and
    // records a self-observation — the writes that platform authority would bar.
    let script = r#"
        local phil = memory.create("person/phil", "Phil, who created me to keep his memory.")
        memory.get("person/operator"):link("same_as", phil)
        memory.get("self"):link("created_by", phil)
        memory.get("self"):append("I exist to keep Phil's memory.", { by_agent = true })
    "#;
    let model = ScriptedModel::new([
        run_lua_call(script),
        Completion::Reply("Hello, Phil. I'll remember.".to_owned()),
        // The two memories that gained content regenerate their descriptions.
        describe_call("Phil, my creator."),
        describe_call("Kestrel, created by Phil."),
        // A later imprint turn, whose freshly-frozen brief we inspect.
        Completion::Reply("Still here.".to_owned()),
    ]);

    let outcome = server
        .control()
        .imprint(
            &model,
            "Hi, I'm Phil. I built you to remember things for me.",
        )
        .await
        .unwrap();
    assert_eq!(
        outcome,
        TurnOutcome::Reply("Hello, Phil. I'll remember.".to_owned())
    );
    // The creator is now a memory of its own (the operator stub was merged into it).
    assert!(server.control().memory("person/phil").unwrap().is_some());

    // A later imprint turn (after the idle gap) opens a fresh session, whose frozen brief surfaces the
    // `created_by` link in the self block — the structural assertion the interview exists to make.
    clock.advance_millis(1_801 * 1_000);
    server
        .control()
        .imprint(&model, "anything else I should know?")
        .await
        .unwrap();
    let sessions = server.control().sessions(&imprint).unwrap();
    let brief = &sessions.last().unwrap().brief;
    assert!(brief.contains("created_by"), "brief was: {brief}");
}

/// A model that distinguishes structured-output (synthesize/describe) calls from conversational step
/// calls: a `response_format` request is a synthesis, answered with a fixed description and its
/// synthesized memory recorded (parsed from the `Memory: <name>` prompt header) so a test can assert
/// which memories a describe pass actually called `generate` for; every other request pops the next
/// scripted conversational step.
struct DispatchingModel {
    steps: Mutex<std::collections::VecDeque<Completion>>,
    synthesized: Mutex<Vec<String>>,
}

impl DispatchingModel {
    fn new(steps: impl IntoIterator<Item = Completion>) -> DispatchingModel {
        DispatchingModel {
            steps: Mutex::new(steps.into_iter().collect()),
            synthesized: Mutex::new(Vec::new()),
        }
    }

    /// The memory handles a synthesis call was made for, in call order.
    fn synthesized(&self) -> Vec<String> {
        self.synthesized.lock().unwrap().clone()
    }
}

/// The fixed description every [`DispatchingModel`] synthesis returns, distinctive enough to assert on
/// in a brief.
const DISPATCH_DESCRIPTION: &str = "A synthesized description from the describer.";

#[async_trait::async_trait]
impl ModelClient for DispatchingModel {
    fn model_id(&self) -> &str {
        "dispatching-model"
    }

    async fn generate(&self, request: &GenerateRequest) -> Result<GenerateResponse, ModelError> {
        // A synthesis is the response-format-constrained call; a step offers tools or a plain reply.
        if request.response_format.is_some() {
            if let Some(name) = request
                .messages
                .first()
                .and_then(|message| message.content.strip_prefix("Memory: "))
                .and_then(|rest| rest.split('\n').next())
            {
                self.synthesized.lock().unwrap().push(name.to_owned());
            }
            return Ok(GenerateResponse {
                completion: Completion::Reply(
                    serde_json::json!({ "description": DISPATCH_DESCRIPTION, "occurrences": [] })
                        .to_string(),
                ),
                usage: Usage::default(),
                reasoning: None,
                finish_reason: None,
            });
        }
        let completion = self
            .steps
            .lock()
            .unwrap()
            .pop_front()
            .ok_or(ModelError::Exhausted)?;
        Ok(GenerateResponse {
            completion,
            usage: Usage::default(),
            reasoning: None,
            finish_reason: None,
        })
    }
}

#[tokio::test]
async fn the_pre_brief_pass_describes_only_the_briefs_memories() {
    // A turn writes a fact about the present participant and an unrelated topic. The next session's
    // pre-brief describe pass is narrowed to the brief's read set, so it describes the participant but
    // not the unrelated topic; a later whole-log catch-up then describes the topic.
    let (server, clock) = born_agent();
    let leads = ConversationLocator::new("discord", "leads");
    let model = DispatchingModel::new([
        run_lua_call(
            r#"memory.get("person/dave"):append("Dave climbs on weekends", { by_agent = true, visibility = "public" })
               local o = memory.create("topic/orphan")
               o:append("An unrelated topic note", { by_agent = true, visibility = "public" })"#,
        ),
        Completion::Reply("noted".to_owned()),
        Completion::Reply("ok".to_owned()),
    ]);

    server
        .platform()
        .route_message(&model, &leads, "dave", "remember this", &["dave"])
        .await
        .unwrap();
    // A fresh session past the idle gap runs the narrowed pre-brief describe over its read set.
    clock.advance_millis(1_801 * 1_000);
    server
        .platform()
        .route_message(&model, &leads, "dave", "again", &["dave"])
        .await
        .unwrap();

    let synthesized = model.synthesized();
    assert!(
        synthesized.iter().any(|name| name == "person/dave"),
        "the present participant is in the brief, so it is described: {synthesized:?}"
    );
    assert!(
        !synthesized.iter().any(|name| name == "topic/orphan"),
        "the unrelated topic is not in the brief, so the narrowed pass skips it: {synthesized:?}"
    );

    // The whole-log catch-up now picks up the topic the narrowed pass left stale.
    let considered = server.describe_catch_up(&model).await.unwrap();
    assert_eq!(considered, 1, "only the orphan topic is still stale");
    assert!(
        model
            .synthesized()
            .iter()
            .any(|name| name == "topic/orphan"),
        "the background catch-up describes the previously-skipped topic"
    );
}

#[tokio::test]
async fn a_prior_turns_write_is_described_before_the_next_briefs_composition() {
    // A fact the first session wrote to the room's context is described at the next session's open,
    // before its brief is composed — so the frozen brief carries the fresh description, not stale prose.
    let (server, clock) = born_agent();
    let leads = ConversationLocator::new("discord", "leads");
    let model = DispatchingModel::new([
        run_lua_call(
            r#"context.current():append("The team is planning a database migration", { by_agent = true, visibility = "public" })"#,
        ),
        Completion::Reply("ok".to_owned()),
        Completion::Reply("ok again".to_owned()),
    ]);

    server
        .platform()
        .route_message(&model, &leads, "dave", "note the room", &["dave"])
        .await
        .unwrap();
    clock.advance_millis(1_801 * 1_000);
    server
        .platform()
        .route_message(&model, &leads, "dave", "back", &["dave"])
        .await
        .unwrap();

    let sessions = server.control().sessions(&leads).unwrap();
    let brief = &sessions.last().unwrap().brief;
    assert!(
        brief.contains(DISPATCH_DESCRIPTION),
        "the second session's brief carries the freshly-synthesized room description: {brief}"
    );
}

#[tokio::test]
async fn a_mid_session_join_catches_the_joiners_description_up_before_the_brief() {
    // The starvation bound on the join-brief: composing a joiner's brief forces the describe
    // catch-up for their memory, so the injected brief reads fresh prose rather than stale.
    let (server, clock) = born_agent();
    let leads = ConversationLocator::new("discord", "leads");
    let model = DispatchingModel::new([
        // Session 1: Erin is present, and the agent writes a public fact about her — left stale
        // for the background describer when the session lapses.
        run_lua_call(
            r#"memory.get("person/erin"):append("Erin runs the design reviews", { by_agent = true, visibility = "public" })"#,
        ),
        Completion::Reply("noted".to_owned()),
        // Session 2, opened by Dave alone: the narrowed pre-brief pass skips absent Erin.
        Completion::Reply("hi dave".to_owned()),
        // Erin's mid-session arrival.
        Completion::Reply("welcome back".to_owned()),
    ]);

    server
        .platform()
        .route_message(
            &model,
            &leads,
            "dave",
            "erin runs the reviews",
            &["dave", "erin"],
        )
        .await
        .unwrap();
    // Past the idle gap, Dave alone opens session 2 — Erin is not in its brief's read set, so her
    // description stays stale.
    clock.advance_millis(1_801 * 1_000);
    server
        .platform()
        .route_message(&model, &leads, "dave", "quiet morning", &["dave"])
        .await
        .unwrap();
    assert!(
        !model.synthesized().iter().any(|name| name == "person/erin"),
        "Erin stays stale while absent: {:?}",
        model.synthesized()
    );

    // Erin arrives mid-session: the join forces the describe catch-up over her memory before her
    // join-brief composes.
    server
        .platform()
        .route_message(
            &model,
            &leads,
            "erin",
            "hey, what did I miss?",
            &["dave", "erin"],
        )
        .await
        .unwrap();
    assert!(
        model.synthesized().iter().any(|name| name == "person/erin"),
        "the join described the stale joiner: {:?}",
        model.synthesized()
    );

    // ...and the injected join-brief carries the fresh description, proving the catch-up ran
    // before the brief composed.
    let erin = server.control().memory("person/erin").unwrap().unwrap().id;
    let events = server.control().events().unwrap();
    let join_brief = events
        .iter()
        .find_map(|event| match &event.payload {
            EventPayload::ConversationTurn {
                role: TurnRole::System,
                participant: Some(participant),
                text,
                ..
            } if *participant == erin => Some(text.clone()),
            _ => None,
        })
        .expect("the join injected a join-brief");
    assert!(
        join_brief.contains(DISPATCH_DESCRIPTION),
        "the join-brief reads the freshly-synthesized description: {join_brief}"
    );
}

#[tokio::test]
async fn a_fresh_genesis_describes_nothing_on_the_first_tick() {
    // Genesis baselines the seeded `self` as already described, so the first describe pass over a
    // fresh agent regenerates nothing and calls the model zero times.
    let (server, _clock) = born_agent();
    let model = DispatchingModel::new([]);
    let considered = server.describe_catch_up(&model).await.unwrap();
    assert_eq!(considered, 0, "nothing is stale after a fresh genesis");
    assert!(
        model.synthesized().is_empty(),
        "the describer made no synthesis calls: {:?}",
        model.synthesized()
    );
}

#[tokio::test]
async fn a_describe_backlog_survives_a_restart() {
    // A memory written but not yet described before shutdown stays stale in the log-derived
    // described-state, so after a rebuild the background describer picks it up — the backlog is not
    // silently dropped at boot.
    let path = std::env::temp_dir().join(format!(
        "zuihitsu-backlog-{}.sqlite",
        MemoryId::generate().0
    ));
    let clock = ManualClock::new(TEST_NOW);
    let leads = ConversationLocator::new("discord", "leads");

    // First process: a turn writes a topic that the pre-brief pass does not describe (it is not in the
    // brief's read set), so it is left stale when the process ends.
    {
        let mut server = Server::new(
            Box::new(SqliteStore::open(&path).unwrap()),
            Graph::open_in_memory().unwrap(),
            Box::new(clock.clone()),
        );
        server.boot().unwrap();
        server.control().create_agent(&seed()).unwrap();
        let model = DispatchingModel::new([
            run_lua_call(
                r#"local m = memory.create("topic/backlog")
                   m:append("A durable fact left undescribed", { by_agent = true, visibility = "public" })"#,
            ),
            Completion::Reply("ok".to_owned()),
        ]);
        server
            .platform()
            .route_message(&model, &leads, "dave", "note this", &["dave"])
            .await
            .unwrap();
        assert!(
            !model
                .synthesized()
                .iter()
                .any(|name| name == "topic/backlog"),
            "the pre-brief pass left the topic undescribed: {:?}",
            model.synthesized()
        );
    } // the server drops: a restart

    // Second process: a fresh graph rebuilt from the same log. The backlog persists, so the describer
    // catches it up.
    let mut server = Server::new(
        Box::new(SqliteStore::open(&path).unwrap()),
        Graph::open_in_memory().unwrap(),
        Box::new(clock.clone()),
    );
    server.boot().unwrap();
    let model = DispatchingModel::new([]);
    let considered = server.describe_catch_up(&model).await.unwrap();
    assert!(
        considered >= 1,
        "the pre-shutdown backlog is described after a restart"
    );
    assert!(
        model
            .synthesized()
            .iter()
            .any(|name| name == "topic/backlog"),
        "the restarted describer picks up the undescribed topic: {:?}",
        model.synthesized()
    );

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
    let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
}

#[tokio::test]
async fn the_buffer_stays_bounded_across_repeated_compactions() {
    // The compaction-seam bug (issue #22): when the turns are small relative to the carryover char
    // budget, the pre-fix carryover tail never trimmed and `from_seq` never advanced, so the live
    // buffer re-spanned every session since the original carryover point — growing without bound. Here
    // the token budget forces a compaction on every message and the char budget is loose (its default),
    // exactly the condition that stuck `from_seq`; the buffer must stay bounded regardless.
    let (server, _clock) = born_agent();
    let mut settings = server.control().settings().unwrap();
    settings.compaction.token_budget = 100;
    // Disable the pre-compaction flush, so each message is a single scripted model step — the buffer
    // growth is isolated from flush turns.
    settings.compaction.flush_min_turns = 1_000_000;
    // A loose char budget (the default) that small turns never fill — the pre-fix stuck condition.
    settings.compaction.carryover_char_budget = 4_000;
    server.control().set_settings(settings).unwrap();

    let leads = ConversationLocator::new("discord", "leads");
    let seams = 8;
    // Every step reports usage over the budget, so every message forces a re-segment.
    let model = ScriptedModel::with_usage(
        (0..seams).map(|i| (Completion::Reply(format!("reply {i}")), 200u32)),
    );

    for i in 0..seams {
        server
            .platform()
            .route_message(&model, &leads, "dave", &format!("message {i}"), &["dave"])
            .await
            .unwrap();
    }

    // Every message re-segmented: one session per message.
    assert_eq!(
        server.control().sessions(&leads).unwrap().len(),
        seams as usize
    );

    let seen = model.recorded_messages();
    assert_eq!(seen.len(), seams as usize);

    // The buffer is bounded: a seeded session sees only the prior session's carried tail plus its own
    // inbound. It must not grow with the number of seams — the pre-fix buffer grew by two messages each
    // seam (reaching 15 at the eighth message), so this bound (four) fails against the old code.
    for (turn_index, messages) in seen.iter().enumerate() {
        assert!(
            messages.len() <= 4,
            "turn {turn_index} buffer grew to {} messages: {:?}",
            messages.len(),
            messages
                .iter()
                .map(|m| m.content.as_str())
                .collect::<Vec<_>>(),
        );
    }

    // From the second seam on, the count is steady, not climbing — the bound is a plateau, not a slower
    // leak.
    let steady = seen[2].len();
    for messages in &seen[2..] {
        assert_eq!(messages.len(), steady, "the bounded buffer size is stable");
    }

    // The trim drops the oldest and keeps the newest: the first message is long gone from the last
    // prompt, and the latest inbound is present.
    let last: Vec<&str> = seen
        .last()
        .unwrap()
        .iter()
        .map(|m| m.content.as_str())
        .collect();
    assert!(
        !last.iter().any(|content| content.contains("message 0")),
        "the original first turn should have been trimmed away, but is still present: {last:?}",
    );
    assert!(
        last.iter()
            .any(|content| content.contains(&format!("message {}", seams - 1))),
        "the newest inbound must always be present: {last:?}",
    );

    // The total char size is bounded too — a generous fixed ceiling (the char budget plus a single
    // session's stamped turns), independent of the seam count; the pre-fix buffer blows past it as the
    // seams accrue.
    let last_chars: usize = seen
        .last()
        .unwrap()
        .iter()
        .map(|m| m.content.chars().count())
        .sum();
    assert!(
        last_chars <= 4_000 + 1_000,
        "the last prompt's char size {last_chars} exceeds the bound",
    );
}

#[tokio::test]
async fn an_arrival_matching_an_unbound_stub_proposes_a_merge_for_the_operator() {
    // An agent-authored hearsay stub: `person/philpax` exists (written from conversation) but is bound
    // to no platform — the operator/agent has never confirmed which platform account it belongs to.
    let (server, _clock) = born_agent();
    let hearsay = MemoryId::generate();
    server
        .control()
        .seed_events(vec![EventPayload::memory_created(
            hearsay,
            Namespace::Person.with_name("philpax"),
        )])
        .unwrap();

    // Philpax then arrives on Discord. The handle matches the unbound stub, so the arrival mints its own
    // platform-qualified stub (it is *not* merged onto the hearsay one from a bare handle match), and an
    // orchestration-sourced merge is proposed to reunite them.
    let model = ScriptedModel::new([Completion::Reply("Hello.".to_owned())]);
    let leads = ConversationLocator::new("discord", "leads");
    server
        .platform()
        .route_message(&model, &leads, "philpax", "hi there", &["philpax"])
        .await
        .unwrap();

    // Both stubs exist and stay distinct: the fresh qualified one and the untouched hearsay one.
    let arrival = server
        .control()
        .memory("person/philpax@discord")
        .unwrap()
        .expect("the arrival minted a platform-qualified stub");
    assert!(server.control().memory("person/philpax").unwrap().is_some());

    // The log carries the orchestration-sourced proposal reuniting the two.
    let proposal = server
        .control()
        .events()
        .unwrap()
        .into_iter()
        .find_map(|event| match event.payload {
            EventPayload::MergeProposed { from, to, source } => Some((from, to, source)),
            _ => None,
        })
        .expect("a merge was proposed for the handle match");
    assert_eq!(
        proposal,
        (arrival.id, hearsay, MergeProposalSource::Orchestration)
    );

    // And it is visible on the operator's merge-proposal surface — unweighed, awaiting the operator —
    // rather than silently dropped or auto-merged.
    let surfaced = server.control().merge_proposals().unwrap();
    assert_eq!(surfaced.len(), 1);
    assert_eq!(surfaced[0].from.as_str(), "person/philpax@discord");
    assert_eq!(surfaced[0].to.as_str(), "person/philpax");
    assert_eq!(surfaced[0].source, MergeProposalSource::Orchestration);
    assert!(!surfaced[0].refused);
}
