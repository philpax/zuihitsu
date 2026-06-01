//! Server tests via the in-process control client: creating an agent, inspecting it, idempotent
//! re-creation, and boot reconciling a fresh graph from a persisted log (spec §Clients, §Storage).

#![cfg(feature = "sqlite")]

use zuihitsu::{
    ConfigValue, Graph, ManualClock, MemoryId, SeedSelf, Server, SqliteStore, Timestamp,
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
    assert_eq!(
        server.control().config("max_steps").unwrap(),
        Some(ConfigValue::Int(12))
    );
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
