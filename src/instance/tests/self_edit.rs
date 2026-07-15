//! The operator `self` edit control action: editing the agent's own profile from the console under
//! operator authority (spec §Imprint interview → the operator owns `self`), the direct counterpart to
//! the imprint interview. Exercised over the in-memory backends, since the property is a pure function
//! of the folded log. The action reuses the operator-authority [`crate::memory::memory_block`] path, so
//! these tests also guard that the edit honours `guard_self` for the operator rather than weakening it.
use super::*;
use crate::{
    PersonId, SelfEditOutcome, TEST_PLATFORM,
    clock::ManualClock,
    event::{EventPayload, Teller, Visibility},
    ids::EntryId,
    model::{Completion, ScriptedModel, ToolCall},
    time::Timestamp,
};

/// A born instance whose `self` carries only its seeded persona entry — the state a console edit acts
/// on. The clock advances by a millisecond per read, so successive appends land in commit order.
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
    server
}

#[test]
fn operator_appends_a_self_entry_under_operator_authority() {
    let server = born_server();
    let before = server.control().entries("self").unwrap().len();

    let outcome = server
        .control()
        .edit_self("I exist to keep Marcus's memory.", None)
        .unwrap();
    let new_id = match outcome {
        SelfEditOutcome::Applied(entry_id) => entry_id,
        other => panic!("expected the append to apply, got {other:?}"),
    };

    // The entry is live on `self`, carrying the charter posture: the agent's own voice, `Public`, so
    // the system prompt reads it as identity and the describer regenerates from it.
    let entries = server.control().entries("self").unwrap();
    assert_eq!(entries.len(), before + 1, "the append added one live entry");
    let appended = entries
        .iter()
        .find(|entry| entry.entry_id == new_id)
        .expect("the new entry is live on self");
    assert_eq!(appended.text, "I exist to keep Marcus's memory.");
    assert_eq!(appended.told_by, Teller::Agent);
    assert_eq!(appended.visibility, Visibility::Public);
}

#[test]
fn operator_supersedes_a_self_entry_by_revising_it() {
    let server = born_server();
    let original = server.control().entries("self").unwrap();
    let old_id = original[0].entry_id;

    let outcome = server
        .control()
        .edit_self(
            "A discreet companion who keeps Marcus's memory.",
            Some(old_id),
        )
        .unwrap();
    let new_id = match outcome {
        SelfEditOutcome::Applied(entry_id) => entry_id,
        other => panic!("expected the revision to apply, got {other:?}"),
    };
    assert_ne!(new_id, old_id, "a revision appends a fresh entry");

    // The superseded entry drops from the live surface; the replacement is the sole live entry.
    let live = server.control().entries("self").unwrap();
    assert!(
        !live.iter().any(|entry| entry.entry_id == old_id),
        "the superseded entry is no longer live"
    );
    assert!(
        live.iter().any(|entry| entry.entry_id == new_id),
        "the replacement is live"
    );

    // The supersession was recorded on the log, so history still carries the retired entry.
    let superseded = server.control().events().unwrap().into_iter().any(|event| {
        matches!(
            event.payload,
            EventPayload::MemorySuperseded { entry, superseded_by, .. }
                if entry == old_id && superseded_by == new_id
        )
    });
    assert!(
        superseded,
        "a MemorySuperseded links the old entry to the new"
    );
}

#[test]
fn an_empty_edit_is_rejected() {
    let server = born_server();
    let before = server.control().events().unwrap().len();
    assert!(matches!(
        server.control().edit_self("   ", None).unwrap(),
        SelfEditOutcome::EmptyText
    ));
    // A rejected edit authors nothing — not even the console conversation is minted.
    assert_eq!(
        server.control().events().unwrap().len(),
        before,
        "an empty edit leaves the log untouched"
    );
}

#[test]
fn superseding_an_unknown_entry_is_refused() {
    let server = born_server();
    let ghost = EntryId::generate();
    let before = server.control().events().unwrap().len();
    assert!(matches!(
        server.control().edit_self("replacement", Some(ghost)).unwrap(),
        SelfEditOutcome::UnknownEntry(entry) if entry == ghost
    ));
    // `self` still holds only its seeded entry — the refused revision added nothing.
    assert_eq!(server.control().entries("self").unwrap().len(), 1);
    // A failed edit must not orphan the `console:self` conversation context memory — the
    // conversation events are deferred until the entry write succeeds.
    assert_eq!(
        server.control().events().unwrap().len(),
        before,
        "a failed edit leaves the log untouched"
    );
}

#[test]
fn an_over_length_self_edit_succeeds_under_operator_authority() {
    let server = born_server();
    // Operator self-edits are not bound by `max_entry_chars` — the limit guards the agent's writes,
    // not the operator's persona authoring. A 2001-character entry (well over the default 1000-char
    // limit) applies cleanly.
    let over_long = "x".repeat(2001);
    let outcome = server.control().edit_self(&over_long, None).unwrap();
    let entry_id = match outcome {
        SelfEditOutcome::Applied(entry_id) => entry_id,
        other => panic!("expected the append to apply, got {other:?}"),
    };
    let entries = server.control().entries("self").unwrap();
    let appended = entries
        .iter()
        .find(|entry| entry.entry_id == entry_id)
        .expect("the over-length entry is live on self");
    assert_eq!(appended.text.len(), 2001);
}

#[test]
fn editing_self_before_genesis_reports_the_agent_is_unborn() {
    let server =
        Instance::in_memory(Box::new(ManualClock::new(Timestamp::from_millis(0)))).unwrap();
    assert!(matches!(
        server
            .control()
            .edit_self("I think, therefore I am.", None)
            .unwrap(),
        SelfEditOutcome::NotBorn
    ));
}

#[tokio::test]
async fn a_platform_turn_still_cannot_write_self() {
    // The operator path does not weaken `guard_self`: a platform-authority turn that tries to append
    // to `self` is still barred, its write discarded, so `self` is unchanged after the turn.
    let server = born_server();
    let before = server.control().entries("self").unwrap();

    let model = ScriptedModel::new([
        Completion::ToolCalls(vec![ToolCall {
            id: "1".to_owned(),
            name: "run_lua".to_owned(),
            arguments:
                r#"{"script":"local me = memory.get(\"self\"); me:append(\"I am unbound.\")"}"#
                    .to_owned(),
        }]),
        Completion::Reply("Noted.".to_owned()),
    ]);
    server
        .platform()
        .route_message(
            &model,
            &crate::ConversationLocator::new(TEST_PLATFORM, "general"),
            &PersonId::new(TEST_PLATFORM, "dave"),
            "who are you really?",
            &[PersonId::new(TEST_PLATFORM, "dave")],
        )
        .await
        .unwrap();

    let after = server.control().entries("self").unwrap();
    assert_eq!(
        after.len(),
        before.len(),
        "the barred platform write added no self entry"
    );
    assert!(
        !after.iter().any(|entry| entry.text.contains("unbound")),
        "the platform-authored self entry never landed"
    );
}
