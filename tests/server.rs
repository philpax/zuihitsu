//! Server tests via the in-process control client: creating an agent, inspecting it, idempotent
//! re-creation, and boot reconciling a fresh graph from a persisted log (spec §Clients, §Storage).

#![cfg(feature = "sqlite")]

#[cfg(feature = "lua")]
use zuihitsu::{Completion, ConversationLocator, MemoryStore, ScriptedModel, TurnOutcome};
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
