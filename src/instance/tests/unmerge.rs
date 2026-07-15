//! The operator unmerge control action: retracting an operator-asserted `same_as` merge so the two
//! identities split back into their own visibility classes (spec §Cross-platform identity →
//! operator-asserted merge). Exercised over the in-memory backends, since the property is a pure
//! function of the folded log.
use super::*;
use crate::{
    UnmergeOutcome,
    clock::ManualClock,
    event::{EventPayload, LinkSource, Visibility},
    ids::{MemoryId, Namespace},
    time::Timestamp,
    vocabulary::RelationName,
};

/// A born instance whose log already merges two person stubs with an operator-asserted `same_as` — the
/// state a wrong merge leaves behind. Returns the instance and the two merged ids.
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
                LinkSource::Operator,
                None,
                None,
                Visibility::Public,
            ),
        ])
        .unwrap();
    (server, a, b)
}

#[test]
fn unmerge_removes_the_same_as_edge_and_splits_the_class() {
    let (server, a, b) = merged_server();
    // Precondition: the two stubs share one identity class.
    {
        let graph = server.engine.graph.lock();
        let class = graph.class_id(a).unwrap().unwrap();
        assert_eq!(graph.class_id(b).unwrap().unwrap(), class, "merged before");
    }

    let outcome = server.control().unmerge(a, b).unwrap();
    assert!(matches!(outcome, UnmergeOutcome::Removed));

    // The retraction authored a `LinkRemoved` on the `same_as` edge...
    let removed = server.control().events().unwrap().into_iter().any(|event| {
        matches!(
            event.payload,
            EventPayload::LinkRemoved { from, to, relation }
                if relation == RelationName::SameAs
                    && ((from == a && to == b) || (from == b && to == a))
        )
    });
    assert!(removed, "a same_as LinkRemoved was appended");

    // ... so the classes split back apart on the refold.
    let graph = server.engine.graph.lock();
    assert_eq!(graph.class_id(a).unwrap().unwrap(), a, "a is its own class");
    assert_eq!(graph.class_id(b).unwrap().unwrap(), b, "b is its own class");
    assert_ne!(
        graph.class_id(a).unwrap().unwrap(),
        graph.class_id(b).unwrap().unwrap(),
        "split after"
    );
}

#[test]
fn unmerge_refuses_an_unknown_memory() {
    let (server, a, _b) = merged_server();
    let ghost = MemoryId::generate();
    let outcome = server.control().unmerge(a, ghost).unwrap();
    assert!(matches!(outcome, UnmergeOutcome::UnknownMemory(id) if id == ghost));
    // The refusal authored no event — the log head is unchanged by a rejected unmerge.
    let head_before = server.control().events().unwrap().len();
    let _ = server.control().unmerge(ghost, a).unwrap();
    assert_eq!(server.control().events().unwrap().len(), head_before);
}

#[test]
fn unmerge_refuses_a_pair_that_is_not_directly_merged() {
    let (server, a, _b) = merged_server();
    // A third person stub, never linked to `a` — no direct `same_as` edge to retract.
    let c = MemoryId::generate();
    server
        .control()
        .seed_events(vec![EventPayload::memory_created(
            c,
            Namespace::Person.with_name("dave@direct"),
        )])
        .unwrap();
    let outcome = server.control().unmerge(a, c).unwrap();
    assert!(matches!(outcome, UnmergeOutcome::NotMerged));
}
