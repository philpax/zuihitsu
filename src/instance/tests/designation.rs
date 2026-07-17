//! The operator primary-designation control action: pinning which stub a `same_as` class resolves
//! through, over the earliest-ULID default (spec §Cross-platform identity). Exercised over the
//! in-memory backends, since the primary is a pure function of the folded log.
use super::*;
use crate::{
    DesignateOutcome,
    clock::ManualClock,
    event::{EventPayload, EventSource, LinkPosture, LinkSource, Visibility},
    ids::{MemoryId, Namespace},
    time::Timestamp,
    vocabulary::RelationName,
};

/// A born instance whose log merges two person stubs (`older`, `newer`) with an operator-asserted
/// `same_as` — the throwaway-then-real-handle case. `older` wins the class by ULID until designated.
fn merged_server() -> (Instance, MemoryId, MemoryId) {
    let server =
        Instance::in_memory(Box::new(ManualClock::new(Timestamp::from_millis(0)))).unwrap();
    server
        .control()
        .create_agent(&crate::SeedSelf {
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
    (server, older, newer)
}

#[test]
fn designate_primary_pins_the_chosen_stub_over_the_earliest_ulid() {
    let (server, older, newer) = merged_server();
    // Precondition: the earliest-ULID stub holds the class.
    {
        let graph = server.engine.graph.lock();
        assert_eq!(graph.class_id(newer).unwrap().unwrap(), older);
    }

    let outcome = server.control().designate_primary(newer, true).unwrap();
    assert!(matches!(outcome, DesignateOutcome::Designated));

    // The designation authored a `ClassPrimaryDesignated`...
    let designated = server.control().events().unwrap().into_iter().any(|event| {
        matches!(
            event.payload,
            EventPayload::ClassPrimaryDesignated { memory, designated }
                if memory == newer && designated
        )
    });
    assert!(designated, "a ClassPrimaryDesignated was appended");

    // ... so the whole class resolves through the pinned stub on the refold.
    let graph = server.engine.graph.lock();
    assert_eq!(graph.class_id(older).unwrap().unwrap(), newer);
    assert_eq!(graph.class_id(newer).unwrap().unwrap(), newer);
    assert!(graph.is_primary_designated(newer).unwrap());
}

#[test]
fn the_log_partitions_by_the_envelope_source() {
    // One log, three authorities: genesis writes as `Bootstrap`, `seed_events` stands in for the
    // agent's own accumulation (`Agent`), and a control action writes as `Operator` — so "show me
    // everything the operator did" is a pure envelope filter.
    let (server, _older, newer) = merged_server();
    server.control().designate_primary(newer, true).unwrap();

    let events = server.control().events().unwrap();
    let genesis_end = events
        .iter()
        .position(|e| matches!(e.payload, EventPayload::GenesisCompleted { .. }))
        .expect("genesis completed");
    assert!(
        events[..=genesis_end]
            .iter()
            .all(|e| e.source == EventSource::Bootstrap)
    );
    assert!(
        events[genesis_end + 1..]
            .iter()
            .filter(|e| !matches!(e.payload, EventPayload::ClassPrimaryDesignated { .. }))
            .all(|e| e.source == EventSource::Agent)
    );
    let designation = events
        .iter()
        .find(|e| matches!(e.payload, EventPayload::ClassPrimaryDesignated { .. }))
        .expect("the designation was appended");
    assert_eq!(designation.source, EventSource::Operator);
}

#[test]
fn releasing_a_designation_restores_the_earliest_ulid_primary() {
    let (server, older, newer) = merged_server();
    server.control().designate_primary(newer, true).unwrap();
    let outcome = server.control().designate_primary(newer, false).unwrap();
    assert!(matches!(outcome, DesignateOutcome::Designated));

    let graph = server.engine.graph.lock();
    assert_eq!(graph.class_id(older).unwrap().unwrap(), older);
    assert!(!graph.is_primary_designated(newer).unwrap());
}

#[test]
fn designate_primary_refuses_an_unknown_memory() {
    let (server, _older, _newer) = merged_server();
    let ghost = MemoryId::generate();
    let outcome = server.control().designate_primary(ghost, true).unwrap();
    assert!(matches!(outcome, DesignateOutcome::UnknownMemory(id) if id == ghost));
    // The refusal authored no event — the log head is unchanged.
    let head_before = server.control().events().unwrap().len();
    let _ = server.control().designate_primary(ghost, true).unwrap();
    assert_eq!(server.control().events().unwrap().len(), head_before);
}
