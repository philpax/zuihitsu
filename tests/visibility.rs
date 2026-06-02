//! Visibility predicate tests (spec appendix scenarios 1, 3, 4, 5, 6, 7–10, 16). Asserts directly on
//! `visible(...)` and `default_visibility(...)` over hand-built memories, entries, present sets, and
//! a `class_of` resolver — deterministic and model-free. The brief-rendering scenarios (2, 13, 14)
//! become meaningful at Stage 8.

#![cfg(feature = "sqlite")]

use std::collections::HashMap;

use zuihitsu::{
    EntryId, GraphError, MarkerRoom, MemoryId, MemoryName, Teller, Timestamp, Visibility,
    Volatility, default_visibility,
    graph::{EntryView, MemoryView},
    room_display, teller_private_marker, visible,
};

fn memory(name: &str) -> MemoryView {
    MemoryView {
        id: MemoryId::generate(),
        name: MemoryName::new(name),
        description: String::new(),
        volatility: Volatility::Medium,
        created_at: Timestamp::from_millis(0),
        tags: Vec::new(),
    }
}

fn entry(told_by: Teller, visibility: Visibility) -> EntryView {
    EntryView {
        entry_id: EntryId::generate(),
        asserted_at: Timestamp::from_millis(0),
        text: "an aside".to_owned(),
        told_by,
        told_in: None,
        visibility,
    }
}

/// The unmerged resolver: every memory is its own class.
fn identity(id: MemoryId) -> Result<MemoryId, GraphError> {
    Ok(id)
}

#[test]
fn teller_private_marker_carries_room_and_confidentiality() {
    // No room known: teller only.
    assert_eq!(
        teller_private_marker("Erin", None),
        "[teller-private, told by Erin]"
    );
    // A known but non-confidential room names the room.
    let general = MarkerRoom {
        name: room_display("context/general"),
        confidential: false,
    };
    assert_eq!(
        teller_private_marker("Erin", Some(&general)),
        "[teller-private, told by Erin in #general]"
    );
    // A #confidential room says so — the cross-context signal the agent reasons over (scenario 13).
    let leads = MarkerRoom {
        name: room_display("context/leads"),
        confidential: true,
    };
    assert_eq!(
        teller_private_marker("Erin", Some(&leads)),
        "[teller-private, told by Erin in #leads (confidential)]"
    );
}

#[test]
fn subject_co_presence_suppresses_the_aside() {
    // Scenario 1: Erin's private aside about Phil, stored on person/phil.
    let phil = memory("person/phil");
    let erin = MemoryId::generate();
    let aside = entry(Teller::Participant(erin), Visibility::PrivateToTeller);

    // (a) Erin alone: surfaces.
    assert!(visible(&aside, &phil, &[erin], &identity).unwrap());
    // (b) Erin and Phil both present: suppressed by the subject-guard.
    assert!(!visible(&aside, &phil, &[erin, phil.id], &identity).unwrap());
}

#[test]
fn self_disclosure_stays_visible_to_its_subject() {
    // Scenario 3: Phil tells the agent something private about himself.
    let phil = memory("person/phil");
    let aside = entry(Teller::Participant(phil.id), Visibility::PrivateToTeller);
    // Subject == teller, so the guard does not fire even with Phil present.
    assert!(visible(&aside, &phil, &[phil.id], &identity).unwrap());
}

#[test]
fn exclude_honours_the_named_party() {
    // Scenario 4: Erin's aside implicating Dave, on a non-person memory so only Exclude guards it.
    let project = memory("project/hooli");
    let (erin, dave, frank) = (
        MemoryId::generate(),
        MemoryId::generate(),
        MemoryId::generate(),
    );
    let aside = entry(Teller::Participant(erin), Visibility::Exclude(vec![dave]));

    assert!(visible(&aside, &project, &[erin], &identity).unwrap()); // (a)
    assert!(!visible(&aside, &project, &[erin, dave], &identity).unwrap()); // (b) excluded present
    assert!(visible(&aside, &project, &[erin, frank], &identity).unwrap()); // (c) Frank isn't excluded
}

#[test]
fn exclude_is_class_aware_across_platforms() {
    // Scenario 5: Exclude({dave@slack}) with dave@slack and dave@discord merged; dave@discord present.
    let project = memory("project/hooli");
    let erin = MemoryId::generate();
    let dave_slack = MemoryId::generate();
    let dave_discord = MemoryId::generate();
    let merged: HashMap<MemoryId, MemoryId> =
        [(dave_slack, dave_slack), (dave_discord, dave_slack)].into();
    let class_of = |id| Ok(*merged.get(&id).unwrap_or(&id));

    let aside = entry(
        Teller::Participant(erin),
        Visibility::Exclude(vec![dave_slack]),
    );
    // dave@discord shares dave's class, so the exclude fires.
    assert!(!visible(&aside, &project, &[erin, dave_discord], &class_of).unwrap());
}

#[test]
fn subject_guard_is_class_aware() {
    // Scenario 6: aside on phil@slack; phil@slack and phil@discord merged; phil@discord present.
    let phil_slack = memory("person/phil@slack");
    let phil_discord = MemoryId::generate();
    let erin = MemoryId::generate();
    let merged: HashMap<MemoryId, MemoryId> = [
        (phil_slack.id, phil_slack.id),
        (phil_discord, phil_slack.id),
    ]
    .into();
    let class_of = |id| Ok(*merged.get(&id).unwrap_or(&id));

    let aside = entry(Teller::Participant(erin), Visibility::PrivateToTeller);
    // The present discord stub shares Phil's class, so the subject-guard suppresses the aside.
    assert!(!visible(&aside, &phil_slack, &[erin, phil_discord], &class_of).unwrap());
}

#[test]
fn unmerged_stubs_do_not_suppress() {
    // Scenario 7: two distinct Phil stubs, unmerged — a different present stub is a different
    // entity, so the subject-guard does not fire (the named cost of operator-only merging).
    let phil_slack = memory("person/phil@slack");
    let phil_discord = MemoryId::generate();
    let erin = MemoryId::generate();
    let aside = entry(Teller::Participant(erin), Visibility::PrivateToTeller);
    assert!(visible(&aside, &phil_slack, &[erin, phil_discord], &identity).unwrap());
}

#[test]
fn non_person_memory_has_no_subject_guard() {
    // Scenario 8: a PrivateToTeller entry on a project is teller-gated only.
    let project = memory("project/hooli");
    let (erin, dave) = (MemoryId::generate(), MemoryId::generate());
    let aside = entry(Teller::Participant(erin), Visibility::PrivateToTeller);
    assert!(visible(&aside, &project, &[erin, dave], &identity).unwrap());
}

#[test]
fn public_is_unconditional() {
    // Scenario 9: a public entry surfaces to anyone, including the subject.
    let phil = memory("person/phil");
    let erin = MemoryId::generate();
    let fact = entry(Teller::Participant(erin), Visibility::Public);
    assert!(visible(&fact, &phil, &[phil.id], &identity).unwrap());
    assert!(visible(&fact, &phil, &[], &identity).unwrap());
}

#[test]
fn agent_authored_content_has_an_ever_present_teller() {
    // Scenario 16: the agent's own observation surfaces — its teller is always present.
    let phil = memory("person/phil");
    let note = entry(Teller::Agent, Visibility::Public);
    assert!(visible(&note, &phil, &[], &identity).unwrap());
    // Even were it private, the agent teller passes; only the subject-guard could suppress it.
    let private = entry(Teller::Agent, Visibility::PrivateToTeller);
    assert!(visible(&private, &phil, &[], &identity).unwrap());
}

#[test]
fn write_time_defaults_follow_the_subject() {
    // Scenario 10: someone else's person memory defaults PrivateToTeller; one's own and any
    // non-person memory default Public.
    let phil = memory("person/phil");
    let erin = MemoryId::generate();
    assert_eq!(
        default_visibility(&phil, &Teller::Participant(erin)),
        Visibility::PrivateToTeller
    );
    assert_eq!(
        default_visibility(&phil, &Teller::Participant(phil.id)),
        Visibility::Public
    );
    assert_eq!(
        default_visibility(&memory("project/hooli"), &Teller::Participant(erin)),
        Visibility::Public
    );
    // Agent-authored content defaults public even on someone else's person memory.
    assert_eq!(
        default_visibility(&phil, &Teller::Agent),
        Visibility::Public
    );
}
