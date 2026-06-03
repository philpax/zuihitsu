//! Server tests via the in-process control client: creating an agent, inspecting it, idempotent
//! re-creation, and boot reconciling a fresh graph from a persisted log (spec §Clients, §Storage).

#![cfg(feature = "sqlite")]

#[cfg(feature = "lua")]
use zuihitsu::{
    Completion, ConversationLocator, MemoryStore, ScriptedModel, ToolCall, TurnOutcome,
};
#[cfg(all(feature = "lua", feature = "openai"))]
use zuihitsu::{EnvConfig, OpenAiClient};
use zuihitsu::{
    Graph, ManualClock, MemoryId, SeedSelf, Server, SqliteStore, Timestamp,
    genesis::{GenesisStatus, Rollout},
};

fn seed() -> SeedSelf {
    SeedSelf {
        agent_name: "Kestrel".to_owned(),
        persona: "A thoughtful, discreet companion with a long memory.".to_owned(),
        seed_entries: vec!["I keep what people tell me in confidence.".to_owned()],
    }
}

fn clock() -> Box<ManualClock> {
    Box::new(ManualClock::new(Timestamp::from_millis(1_000)))
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

/// Install a test-scoped tracing subscriber so the model-gated smoke emits structured, timestamped
/// logs (visible under `--nocapture`) rather than ad-hoc prints. Idempotent across the binary.
#[cfg(all(feature = "lua", feature = "openai"))]
fn init_tracing() {
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_max_level(tracing::Level::DEBUG)
        .try_init();
}

/// A born agent over an in-memory store, returning a clock handle (sharing the boxed clock's atomic)
/// so a test can advance time.
#[cfg(feature = "lua")]
fn born_agent() -> (Server, ManualClock) {
    let clock = ManualClock::new(Timestamp::from_millis(1_000));
    let mut server = Server::new(
        Box::new(MemoryStore::new()),
        Graph::open_in_memory().unwrap(),
        Box::new(clock.clone()),
    );
    server.control().create_agent(&seed()).unwrap();
    (server, clock)
}

#[cfg(feature = "lua")]
#[tokio::test]
async fn route_message_opens_a_session_and_runs_a_turn() {
    let (mut server, _clock) = born_agent();
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
    assert!(
        server
            .control()
            .memory("person/dave@discord")
            .unwrap()
            .is_some()
    );

    // One session opened, carrying a frozen, non-empty brief.
    let sessions = server.control().sessions(&leads).unwrap();
    assert_eq!(sessions.len(), 1);
    assert!(!sessions[0].brief.is_empty());
}

#[cfg(feature = "lua")]
#[tokio::test]
async fn a_session_is_reused_within_the_idle_gap_and_reopened_after() {
    let (mut server, clock) = born_agent();
    let model = ScriptedModel::new([
        Completion::Reply("one".to_owned()),
        Completion::Reply("two".to_owned()),
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

#[cfg(feature = "lua")]
#[tokio::test]
async fn note_join_records_the_arriving_participant_on_the_session() {
    let (mut server, _clock) = born_agent();
    let model = ScriptedModel::new([Completion::Reply("hi".to_owned())]);
    let leads = ConversationLocator::new("discord", "leads");

    // Open a session with Dave present.
    server
        .platform()
        .route_message(&model, &leads, "dave", "hi", &["dave"])
        .await
        .unwrap();
    let dave = server
        .control()
        .memory("person/dave@discord")
        .unwrap()
        .unwrap()
        .id;

    // Erin joins mid-session: she is recorded on the session, alongside Dave.
    server.platform().note_join(&leads, "erin").unwrap();
    let erin = server
        .control()
        .memory("person/erin@discord")
        .unwrap()
        .unwrap()
        .id;

    let sessions = server.control().sessions(&leads).unwrap();
    assert_eq!(sessions.len(), 1);
    let participants = &sessions[0].participants;
    assert!(participants.contains(&dave));
    assert!(participants.contains(&erin));
}

#[cfg(feature = "lua")]
#[tokio::test]
async fn a_token_budget_crossing_forces_a_re_segment_within_the_idle_gap() {
    let (mut server, _clock) = born_agent();
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

#[cfg(feature = "lua")]
#[tokio::test]
async fn the_live_buffer_is_replayed_to_the_model_on_later_turns() {
    let (mut server, _clock) = born_agent();
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
    // Turn 1's prompt is just the inbound message.
    let turn1: Vec<&str> = seen[0]
        .iter()
        .map(|message| message.content.as_str())
        .collect();
    assert_eq!(turn1, vec!["hello there"]);
    // Turn 2 replays the live buffer — turn 1's participant and agent turns — then the new inbound.
    let turn2: Vec<&str> = seen[1]
        .iter()
        .map(|message| message.content.as_str())
        .collect();
    assert_eq!(turn2, vec!["hello there", "first reply", "and again"]);
}

#[cfg(feature = "lua")]
fn run_lua_call(script: &str) -> Completion {
    Completion::ToolCalls(vec![ToolCall {
        id: "lua".to_owned(),
        name: "run_lua".to_owned(),
        arguments: serde_json::json!({ "script": script }).to_string(),
    }])
}

#[cfg(feature = "lua")]
fn describe_call(description: &str) -> Completion {
    Completion::ToolCalls(vec![ToolCall {
        id: "describe".to_owned(),
        name: "describe".to_owned(),
        arguments: serde_json::json!({ "description": description }).to_string(),
    }])
}

#[cfg(feature = "lua")]
#[tokio::test]
async fn a_substantive_session_flushes_to_memory_before_the_cut() {
    let (mut server, _clock) = born_agent();
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

    // Only the flush calls run_lua here, so the memory's presence is the flush's signature — it ran
    // on the hot path and wrote the working state to memory, with its description regenerated.
    let plan = server.control().memory("topic/plan").unwrap();
    assert!(plan.is_some());
    assert_eq!(
        plan.unwrap().description,
        "The team's plan to ship on Friday."
    );
}

#[cfg(feature = "lua")]
#[tokio::test]
async fn a_low_activity_session_skips_the_flush() {
    let (mut server, _clock) = born_agent();
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

#[cfg(feature = "lua")]
#[tokio::test]
async fn the_working_set_carries_into_the_next_session_brief() {
    let (mut server, _clock) = born_agent();
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

#[cfg(feature = "lua")]
#[tokio::test]
async fn an_active_in_thread_carries_across_a_compaction_even_when_untouched() {
    let (mut server, clock) = born_agent();
    let mut settings = server.control().settings().unwrap();
    settings.compaction.token_budget = 100;
    server.control().set_settings(settings).unwrap();

    let leads = ConversationLocator::new("discord", "leads");
    let model = ScriptedModel::with_usage([
        // Session 1: flag a thread active_in the room (an ordinary turn, under budget).
        (
            run_lua_call(
                r#"local t = memory.create("topic/migration", "Plan the DB migration"); t:link("active_in", context.current())"#,
            ),
            10,
        ),
        (Completion::Reply("flagged".to_owned()), 0),
        (describe_call("The DB migration plan."), 0),
        // Session 2 (after an idle reopen) crosses the budget without touching the migration thread.
        (Completion::Reply("on something else".to_owned()), 200),
        // Session 3 opens with the carryover; its frozen brief is what we inspect.
        (Completion::Reply("back".to_owned()), 0),
    ]);

    server
        .platform()
        .route_message(&model, &leads, "dave", "plan the migration", &["dave"])
        .await
        .unwrap();
    // An idle gap reopens a fresh session 2 (no carryover, and it will not touch the thread).
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

    // Session 2 never touched the migration thread, yet it carries into session 3's brief — proving
    // active_in is a distinct, persistent working-set source, not just an alias for touch-derivation.
    let sessions = server.control().sessions(&leads).unwrap();
    let latest = sessions.last().unwrap();
    assert!(
        latest.brief.contains("topic/migration"),
        "brief was: {}",
        latest.brief
    );
}

/// End-to-end smoke against the configured model: route a real message through the whole pipeline —
/// resolve, open a session and freeze its brief, run the loop, reply — and observe what the agent
/// did. Ignored by default (needs a reachable endpoint from `config.toml`); the client has no request
/// timeout, so a slow cold start is tolerated.
#[cfg(all(feature = "lua", feature = "openai"))]
#[tokio::test]
#[ignore = "requires a reachable model endpoint (config.toml)"]
async fn real_model_routes_a_message_end_to_end() {
    init_tracing();
    let Ok(config) = EnvConfig::load(std::path::Path::new("config.toml")) else {
        tracing::warn!("skipping: no config.toml");
        return;
    };
    if config.model.endpoint.is_empty() {
        tracing::warn!("skipping: no model endpoint configured");
        return;
    }
    let client = OpenAiClient::new(&config.model);
    let (mut server, _clock) = born_agent();
    let leads = ConversationLocator::new("direct", "operator");

    let outcome = server
        .platform()
        .route_message(
            &client,
            &leads,
            "operator",
            "Please remember that Dave climbs at the bouldering gym, then confirm you've noted it.",
            &["operator"],
        )
        .await;

    match outcome {
        Ok(outcome) => {
            tracing::info!(?outcome, "real-model route outcome");
            // The full pipeline ran: a single session opened, carrying a frozen brief.
            let sessions = server.control().sessions(&leads).unwrap();
            assert_eq!(sessions.len(), 1);
            assert!(!sessions[0].brief.is_empty());
            // Observe whatever the agent chose to write (a real model names memories variously).
            for namespace in ["person/", "topic/", "place/"] {
                for memory in server.control().memories(namespace).unwrap() {
                    tracing::info!(name = %memory.name.as_str(), description = %memory.description, "agent wrote memory");
                }
            }
        }
        Err(error) => tracing::warn!(%error, "skipping"),
    }
}
