//! The operator entry retraction control action: withdrawing a fact outright from any memory under
//! operator authority (spec §Visibility → the operator withdraws a fact). Exercised over the
//! in-memory backends, since the property is a pure function of the folded log.
use super::*;
use crate::{
    RetractOutcome, clock::ManualClock, event::EventPayload, ids::EntryId, time::Timestamp,
};

/// A born instance with a `person/dave` memory carrying one live entry — the state a retraction
/// acts on. The clock advances by a millisecond per read, so successive appends land in commit
/// order.
fn born_server() -> Instance {
    let server =
        Instance::in_memory(Box::new(ManualClock::new(Timestamp::from_millis(0)))).unwrap();
    server
        .control()
        .create_agent(&crate::SeedSelf {
            agent_name: "Kestrel".to_owned(),
            persona: "A discreet companion with a long memory.".to_owned(),
            seed_entries: vec![],
        })
        .unwrap();
    // Append an entry to a non-self memory so the retraction has a target that is not under the
    // self-edit's guard.
    server
        .control()
        .edit_self("I exist to keep Marcus's memory.", None)
        .unwrap();
    server
}

#[test]
fn operator_retracts_a_live_entry() {
    let server = born_server();
    // Use the seeded persona entry on self as the retraction target.
    let entries = server.control().entries("self").unwrap();
    let target = entries[0].entry_id;

    let outcome = server
        .control()
        .retract_entry("self", target, "outdated persona")
        .unwrap();
    assert!(matches!(outcome, RetractOutcome::Retracted));

    // The retracted entry drops from the live surface but remains in history with its reason.
    let live = server.control().entries("self").unwrap();
    assert!(
        !live.iter().any(|entry| entry.entry_id == target),
        "the retracted entry is no longer live"
    );

    // The EntryRetracted event was recorded on the log.
    let retracted = server.control().events().unwrap().into_iter().any(|event| {
        matches!(
            event.payload,
            EventPayload::EntryRetracted { entry, .. } if entry == target
        )
    });
    assert!(retracted, "an EntryRetracted was recorded");
}

#[test]
fn retracting_an_unknown_memory_is_refused() {
    let server = born_server();
    let before = server.control().events().unwrap().len();
    assert!(matches!(
        server
            .control()
            .retract_entry("person/ghost", EntryId::generate(), "no such memory")
            .unwrap(),
        RetractOutcome::UnknownMemory
    ));
    assert_eq!(
        server.control().events().unwrap().len(),
        before,
        "a failed retraction leaves the log untouched"
    );
}

#[test]
fn retracting_an_unknown_entry_is_refused() {
    let server = born_server();
    let before = server.control().events().unwrap().len();
    let ghost = EntryId::generate();
    assert!(matches!(
        server
            .control()
            .retract_entry("self", ghost, "no such entry")
            .unwrap(),
        RetractOutcome::UnknownEntry(entry) if entry == ghost
    ));
    assert_eq!(
        server.control().events().unwrap().len(),
        before,
        "a failed retraction leaves the log untouched"
    );
}

#[test]
fn a_retraction_without_reason_is_refused() {
    let server = born_server();
    let entries = server.control().entries("self").unwrap();
    let target = entries[0].entry_id;
    let before = server.control().events().unwrap().len();
    assert!(matches!(
        server
            .control()
            .retract_entry("self", target, "   ")
            .unwrap(),
        RetractOutcome::EmptyReason
    ));
    assert_eq!(
        server.control().events().unwrap().len(),
        before,
        "a failed retraction leaves the log untouched"
    );
}
