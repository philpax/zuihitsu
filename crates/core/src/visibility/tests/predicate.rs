//! The entry predicate: postures, presence, subject guards, class awareness, and defaults.

use super::*;

#[test]
fn attributed_is_visible_regardless_of_who_is_present() {
    // An ordinary fact Erin relayed about Dave, classified Attributed: it survives Erin's absence
    // (unlike PrivateToTeller), so the agent can answer about Dave to anyone, later, anywhere.
    let dave = memory("person/dave");
    let erin = MemoryId::generate();
    let stranger = MemoryId::generate();
    let fact = entry(Teller::Participant(erin), Visibility::Attributed);
    // Teller absent, a stranger present, no one present: visible in every case.
    assert!(visible(&fact, &dave, &[stranger], &identity).unwrap());
    assert!(visible(&fact, &dave, &[], &identity).unwrap());
    // And visible even to the subject — it is not a confidence to hold from Dave.
    assert!(visible(&fact, &dave, &[dave.id], &identity).unwrap());
}

#[test]
fn a_superseded_entry_is_never_visible() {
    // A public fact that would otherwise surface to anyone present is suppressed once superseded
    // (spec §Visibility → superseded entries are not live). This guards the search path, which
    // resolves a vector hit through `entry_by_id` before the predicate.
    let dave = memory("person/dave");
    let mut fact = entry(Teller::Agent, Visibility::Public);
    assert!(visible(&fact, &dave, &[], &identity).unwrap());
    fact.superseded_by = Some(EntryId::generate());
    assert!(!visible(&fact, &dave, &[], &identity).unwrap());
}

#[test]
fn a_retracted_entry_is_never_visible() {
    // A retraction tombstones an entry by stamping its own id into `superseded_by` and recording a
    // reason, so the same not-live predicate hides it from every surface — including the search path,
    // which resolves a vector hit before this predicate.
    let dave = memory("person/dave");
    let mut fact = entry(Teller::Agent, Visibility::Public);
    assert!(visible(&fact, &dave, &[], &identity).unwrap());
    fact.superseded_by = Some(fact.entry_id);
    fact.retracted_reason = Some("filed on the wrong person".to_owned());
    assert!(!visible(&fact, &dave, &[], &identity).unwrap());
}

#[test]
fn subject_co_presence_suppresses_the_aside() {
    // Scenario 1: Erin's private aside about Marcus, stored on person/marcus.
    let marcus = memory("person/marcus");
    let erin = MemoryId::generate();
    let aside = entry(Teller::Participant(erin), Visibility::PrivateToTeller);

    // (a) Erin alone: surfaces.
    assert!(visible(&aside, &marcus, &[erin], &identity).unwrap());
    // (b) Erin and Marcus both present: suppressed by the subject-guard.
    assert!(!visible(&aside, &marcus, &[erin, marcus.id], &identity).unwrap());
}

#[test]
fn self_disclosure_stays_visible_to_its_subject() {
    // Scenario 3: Marcus tells the agent something private about himself.
    let marcus = memory("person/marcus");
    let aside = entry(Teller::Participant(marcus.id), Visibility::PrivateToTeller);
    // Subject == teller, so the guard does not fire even with Marcus present.
    assert!(visible(&aside, &marcus, &[marcus.id], &identity).unwrap());
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
    let aside = entry(
        Teller::Participant(erin),
        Visibility::Exclude(BTreeSet::from([dave])),
    );

    assert!(visible(&aside, &project, &[erin], &identity).unwrap()); // (a)
    assert!(!visible(&aside, &project, &[erin, dave], &identity).unwrap()); // (b) excluded present
    assert!(visible(&aside, &project, &[erin, frank], &identity).unwrap()); // (c) Frank isn't excluded
}

#[test]
fn exclude_is_class_aware_across_platforms() {
    // Scenario 5: Exclude({dave@forum}) with dave@forum and dave@chat merged; dave@chat present.
    let project = memory("project/hooli");
    let erin = MemoryId::generate();
    let dave_forum = MemoryId::generate();
    let dave_chat = MemoryId::generate();
    let merged: HashMap<MemoryId, MemoryId> =
        [(dave_forum, dave_forum), (dave_chat, dave_forum)].into();
    let class_of = |id| Ok(*merged.get(&id).unwrap_or(&id));

    let aside = entry(
        Teller::Participant(erin),
        Visibility::Exclude(BTreeSet::from([dave_forum])),
    );
    // dave@chat shares dave's class, so the exclude fires.
    assert!(!visible(&aside, &project, &[erin, dave_chat], &class_of).unwrap());
}

#[test]
fn subject_guard_is_class_aware() {
    // Scenario 6: aside on marcus@forum; marcus@forum and marcus@chat merged; marcus@chat present.
    let marcus_forum = memory("person/marcus@forum");
    let marcus_chat = MemoryId::generate();
    let erin = MemoryId::generate();
    let merged: HashMap<MemoryId, MemoryId> = [
        (marcus_forum.id, marcus_forum.id),
        (marcus_chat, marcus_forum.id),
    ]
    .into();
    let class_of = |id| Ok(*merged.get(&id).unwrap_or(&id));

    let aside = entry(Teller::Participant(erin), Visibility::PrivateToTeller);
    // The present chat stub shares Marcus's class, so the subject-guard suppresses the aside.
    assert!(!visible(&aside, &marcus_forum, &[erin, marcus_chat], &class_of).unwrap());
}

#[test]
fn unmerged_stubs_do_not_suppress() {
    // Scenario 7: two distinct Marcus stubs, unmerged — a different present stub is a different
    // entity, so the subject-guard does not fire (the named cost of operator-only merging).
    let marcus_forum = memory("person/marcus@forum");
    let marcus_chat = MemoryId::generate();
    let erin = MemoryId::generate();
    let aside = entry(Teller::Participant(erin), Visibility::PrivateToTeller);
    assert!(visible(&aside, &marcus_forum, &[erin, marcus_chat], &identity).unwrap());
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
fn non_person_public_facts_survive_the_tellers_absence() {
    // Scenario 12: a Public fact on a non-person memory, relayed by a participant who is now absent,
    // still surfaces to whoever is present. Non-person knowledge (a project, a topic) defaults Public
    // and does not fragment by teller-presence the way a person-memory confidence does, so
    // project and topic knowledge stays discussable no matter who told it.
    let project = memory("project/hooli");
    let erin = MemoryId::generate();
    let stranger = MemoryId::generate();
    let fact = entry(Teller::Participant(erin), Visibility::Public);
    // Teller absent, a different party present: still visible.
    assert!(visible(&fact, &project, &[stranger], &identity).unwrap());
    // And visible with no one present at all.
    assert!(visible(&fact, &project, &[], &identity).unwrap());
}

#[test]
fn public_is_unconditional() {
    // Scenario 9: a public entry surfaces to anyone, including the subject.
    let marcus = memory("person/marcus");
    let erin = MemoryId::generate();
    let fact = entry(Teller::Participant(erin), Visibility::Public);
    assert!(visible(&fact, &marcus, &[marcus.id], &identity).unwrap());
    assert!(visible(&fact, &marcus, &[], &identity).unwrap());
}

#[test]
fn agent_authored_content_has_an_ever_present_teller() {
    // Scenario 16: the agent's own observation surfaces — its teller is always present.
    let marcus = memory("person/marcus");
    let note = entry(Teller::Agent, Visibility::Public);
    assert!(visible(&note, &marcus, &[], &identity).unwrap());
    // Even were it private, the agent teller passes; only the subject-guard could suppress it.
    let private = entry(Teller::Agent, Visibility::PrivateToTeller);
    assert!(visible(&private, &marcus, &[], &identity).unwrap());
}

#[test]
fn write_time_defaults_follow_the_subject() {
    // Scenario 10: someone else's person memory defaults PrivateToTeller; one's own and any
    // non-person memory default Public.
    let marcus = memory("person/marcus");
    let erin = MemoryId::generate();
    assert_eq!(
        default_visibility(&marcus, &Teller::Participant(erin)),
        Visibility::PrivateToTeller
    );
    assert_eq!(
        default_visibility(&marcus, &Teller::Participant(marcus.id)),
        Visibility::Public
    );
    assert_eq!(
        default_visibility(&memory("project/hooli"), &Teller::Participant(erin)),
        Visibility::Public
    );
    // Agent-authored content defaults public even on someone else's person memory.
    assert_eq!(
        default_visibility(&marcus, &Teller::Agent),
        Visibility::Public
    );
}
