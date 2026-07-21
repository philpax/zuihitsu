use super::*;
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
        server.control().settings().unwrap().turn.max_steps,
        zuihitsu::TurnSettings::default().max_steps,
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
    // The whole log is snapshotted before the instance drops, then carried in memory into a fresh
    // instance — a restart that keeps the persisted log but resets runtime state.
    let log = {
        let mut server = Server::new(
            Box::new(MemoryStore::new()),
            Graph::open_in_memory().unwrap(),
            clock(),
        );
        server.boot().unwrap();
        server.control().create_agent(&seed()).unwrap();
        server.control().events().unwrap()
    }; // the store (and its log lock) drop here

    {
        // Reopen the persisted log with a brand-new, empty graph: boot must catch it up to
        // log-head before the agent is inspectable.
        let mut server = Server::new(
            Box::new(MemoryStore::from_events(log)),
            Graph::open_in_memory().unwrap(),
            clock(),
        );
        assert_eq!(server.boot().unwrap(), GenesisStatus::Complete);
        assert!(server.control().memory("self").unwrap().is_some());
    }
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
