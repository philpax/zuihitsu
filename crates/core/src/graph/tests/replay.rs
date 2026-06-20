use super::{materialized, recovery_log};
use crate::{
    event::{EventPayload, Teller, Visibility, Volatility},
    graph::Graph,
    ids::{
        ConversationId, ConversationLocator, EntryId, MemoryId, MemoryName, Namespace, Seq,
        SessionId, TurnId,
    },
    store::{MemoryStore, Store},
    time::Timestamp,
};

#[test]
fn a_snapshot_round_trips_the_graph_and_its_head() {
    let id = MemoryId::generate();
    let (store, graph) = materialized(vec![
        EventPayload::MemoryCreated {
            id,
            name: Namespace::Person.with_name("dave").into(),
        },
        EventPayload::MemoryContentAppended {
            id,
            entry_id: EntryId::generate(),
            asserted_at: Timestamp::from_millis(900),
            occurred_at: None,
            text: "Met at the climbing gym".to_owned(),
            told_by: Teller::Agent,
            told_in: None,
            visibility: Visibility::Public,
        },
    ]);
    let head = graph.head().unwrap();
    assert!(head > Seq::ZERO);

    // VACUUM INTO a fresh file, then open it as a graph: its whole logical state round-trips.
    let path = std::env::temp_dir().join(format!(
        "zuihitsu-graphsnap-{}.sqlite",
        MemoryId::generate().0
    ));
    graph.snapshot_into(&path).unwrap();
    let mut restored = Graph::open(&path).unwrap();
    assert_eq!(restored.head().unwrap(), head);
    // The content fingerprint matches exactly — the entire logical state round-tripped, not just the
    // few fields a spot check would cover.
    assert_eq!(
        restored.fingerprint().unwrap(),
        graph.fingerprint().unwrap()
    );
    // Materializing the restored graph against the same log is a no-op — it is already at head, so a
    // boot from this snapshot replays only the (empty here) tail rather than the whole log.
    assert_eq!(restored.materialize_from(&store).unwrap(), 0);

    std::fs::remove_file(&path).unwrap();
}

#[test]
fn fingerprint_equals_for_identical_state_and_differs_on_change() {
    let id = MemoryId::generate();
    let base = vec![
        EventPayload::MemoryCreated {
            id,
            name: Namespace::Person.with_name("dave").into(),
        },
        EventPayload::MemoryContentAppended {
            id,
            entry_id: EntryId::generate(),
            asserted_at: Timestamp::from_millis(900),
            occurred_at: None,
            text: "Met at the gym".to_owned(),
            told_by: Teller::Agent,
            told_in: None,
            visibility: Visibility::Public,
        },
    ];

    // Two graphs materialized from the same events (same ids) fingerprint identically.
    let (_store_a, a) = materialized(base.clone());
    let (_store_b, b) = materialized(base.clone());
    assert_eq!(a.fingerprint().unwrap(), b.fingerprint().unwrap());

    // One more event — and the head it advances — diverges the fingerprint.
    let mut more = base;
    more.push(EventPayload::MemoryVolatilitySet {
        id,
        volatility: Volatility::High,
    });
    let (_store_c, c) = materialized(more);
    assert_ne!(a.fingerprint().unwrap(), c.fingerprint().unwrap());
}

#[test]
fn replay_is_deterministic_over_a_rich_log() {
    // The same log materialized twice must produce identical projected state — the determinism a
    // rebuild-from-the-log promise rests on (spec §Storage, §Known limitations → storage-layer
    // corruption). A rich log exercises most handlers, where the existing fingerprint test uses a
    // two-event log.
    let log = recovery_log();
    let (_a_store, a) = materialized(log.clone());
    let (_b_store, b) = materialized(log);
    assert_eq!(a.fingerprint().unwrap(), b.fingerprint().unwrap());
}

#[test]
fn a_snapshot_plus_a_nonempty_tail_equals_a_full_replay() {
    // The catch-up correctness behind cheap rebuild (spec §Storage → snapshots): a snapshot captured
    // at seq N, plus the log tail replayed from it, is identical to a full replay from seq 0. The
    // round-trip test replays only an empty tail; this one replays real events on top of the snapshot.
    let log = recovery_log();
    let split = log.len() / 2;

    let temp = |tag: &str| {
        std::env::temp_dir().join(format!("zuihitsu-{tag}-{}.sqlite", MemoryId::generate().0))
    };
    let cleanup = |path: std::path::PathBuf| {
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
        let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
    };

    // Materialize the first half into a file graph, then snapshot it at that head.
    let mut store = MemoryStore::new();
    store
        .append(Timestamp::from_millis(1_000), log[..split].to_vec())
        .unwrap();
    let graph_path = temp("recovery-graph");
    let snap_path = temp("recovery-snap");
    {
        let mut head_graph = Graph::open(&graph_path).unwrap();
        head_graph.materialize_from(&store).unwrap();
        head_graph.snapshot_into(&snap_path).unwrap();
    }

    // The rest of the log arrives after the snapshot.
    store
        .append(Timestamp::from_millis(2_000), log[split..].to_vec())
        .unwrap();

    // Restore from the snapshot (copy it over a graph path) and replay only the tail.
    let restored_path = temp("recovery-restored");
    std::fs::copy(&snap_path, &restored_path).unwrap();
    let mut restored = Graph::open(&restored_path).unwrap();
    let replayed = restored.materialize_from(&store).unwrap();
    assert_eq!(replayed, log.len() - split, "only the tail should replay");

    // A full replay from seq 0 into a fresh graph must reach byte-identical projected state.
    let mut full = Graph::open_in_memory().unwrap();
    full.materialize_from(&store).unwrap();
    assert_eq!(restored.fingerprint().unwrap(), full.fingerprint().unwrap());

    cleanup(graph_path);
    cleanup(snap_path);
    cleanup(restored_path);
}

#[test]
fn materialize_is_incremental() {
    let id = MemoryId::generate();
    let mut store = MemoryStore::new();
    let mut graph = Graph::open_in_memory().unwrap();

    store
        .append(
            Timestamp::from_millis(1),
            vec![EventPayload::MemoryCreated {
                id,
                name: MemoryName::new("concept/recursion"),
            }],
        )
        .unwrap();
    assert_eq!(graph.materialize_from(&store).unwrap(), 1);
    assert_eq!(graph.head().unwrap(), Seq(1));

    // A second pass with no new events applies nothing and leaves the head where it was.
    assert_eq!(graph.materialize_from(&store).unwrap(), 0);

    store
        .append(
            Timestamp::from_millis(2),
            vec![EventPayload::MemoryDescriptionRegenerated {
                id,
                new_text: "A function defined in terms of itself.".to_owned(),
                produced_by: None,
            }],
        )
        .unwrap();
    assert_eq!(graph.materialize_from(&store).unwrap(), 1);
    assert_eq!(graph.head().unwrap(), Seq(2));
    assert_eq!(
        graph.memory_by_id(id).unwrap().unwrap().description,
        "A function defined in terms of itself."
    );
}

#[test]
fn conversations_and_sessions_project() {
    let conv = ConversationId::generate();
    let context = MemoryId::generate();
    let (s1, s2) = (SessionId::generate(), SessionId::generate());
    let alice = MemoryId::generate();
    let bob = MemoryId::generate();
    let carol = MemoryId::generate();
    let join_turn = TurnId::generate();
    let (_store, graph) = materialized(vec![
        EventPayload::MemoryCreated {
            id: context,
            name: Namespace::Context
                .with_name("discord:guild/42/chan/leads")
                .into(),
        },
        EventPayload::ConversationStarted {
            id: conv,
            locator: ConversationLocator::new("discord", "guild/42/chan/leads"),
            context_memory: context,
        },
        EventPayload::SessionStarted {
            conversation: conv,
            id: s1,
            participants: vec![alice, bob],
            started_at: Timestamp::from_millis(1_000),
            seeded_from_turn: None,
            brief: "first brief".to_owned(),
        },
        EventPayload::ParticipantJoined {
            conversation: conv,
            session: s1,
            participant: carol,
            at_turn: join_turn,
        },
        EventPayload::SessionEnded {
            conversation: conv,
            id: s1,
        },
        // A second session opened via compaction carries the carryover extent.
        EventPayload::SessionStarted {
            conversation: conv,
            id: s2,
            participants: vec![alice],
            started_at: Timestamp::from_millis(5_000),
            seeded_from_turn: Some(join_turn),
            brief: "second brief".to_owned(),
        },
    ]);

    // The locator resolves to the room; an unseen locator does not.
    assert_eq!(
        graph
            .conversation_for_locator(&ConversationLocator::new("discord", "guild/42/chan/leads"))
            .unwrap(),
        Some(conv)
    );
    assert!(
        graph
            .conversation_for_locator(&ConversationLocator::new("discord", "elsewhere"))
            .unwrap()
            .is_none()
    );
    // The room resolves to its eagerly-minted context memory.
    assert_eq!(graph.context_for_conversation(conv).unwrap(), Some(context));

    // Sessions project in commit order, carrying the brief and the carryover extent.
    let sessions = graph.sessions_in(conv).unwrap();
    assert_eq!(sessions.len(), 2);
    assert_eq!(sessions[0].id, s1);
    assert_eq!(sessions[0].brief, "first brief");
    assert_eq!(sessions[0].seeded_from_turn, None);
    assert_eq!(sessions[1].id, s2);
    assert_eq!(sessions[1].seeded_from_turn, Some(join_turn));

    // The first session's participants are the open set plus the mid-session joiner.
    let mut expected = vec![alice, bob, carol];
    expected.sort();
    assert_eq!(graph.session_participants(s1).unwrap(), expected);
    assert_eq!(graph.session(s1).unwrap().unwrap().participants, expected);
    // The second session has only its open participant.
    assert_eq!(graph.session_participants(s2).unwrap(), vec![alice]);
}
