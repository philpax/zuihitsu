//! Visibility predicate tests (spec appendix scenarios 1, 3, 4, 5, 6, 7–10, 16). Asserts directly
//! on `visible(...)` and `default_visibility(...)` over hand-built memories, entries, present
//! sets, and a `class_of` resolver — deterministic and model-free.
use std::collections::HashMap;

use super::{
    MarkerRoom, default_link_visibility, default_visibility, link_explain, link_marker,
    link_visible, room_display, teller_private_marker, visible,
};
use crate::{
    event::{Teller, Visibility, Volatility},
    graph::{EntryView, GraphError, LinkVis, MemoryView},
    ids::{EntryId, MemoryId, MemoryName},
    time::Timestamp,
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
        occurred_sort: None,
        occurred_at: None,
        occurred_authored: false,
        text: "an aside".to_owned(),
        told_by,
        told_in: None,
        visibility,
        superseded_by: None,
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
fn entry_marker_picks_the_register_by_posture() {
    use super::entry_marker;
    let general = MarkerRoom {
        name: room_display("context/general"),
        confidential: false,
    };
    // Public carries no marker; attributed the lighter provenance register; a confidence the
    // teller-private register.
    assert_eq!(
        entry_marker(&Visibility::Public, "Erin", Some(&general)),
        None
    );
    assert_eq!(
        entry_marker(&Visibility::Attributed, "Erin", Some(&general)),
        Some("[via Erin in #general]".to_owned())
    );
    assert_eq!(
        entry_marker(&Visibility::Attributed, "Erin", None),
        Some("[via Erin]".to_owned())
    );
    assert_eq!(
        entry_marker(&Visibility::PrivateToTeller, "Erin", None),
        Some("[teller-private, told by Erin]".to_owned())
    );
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
    // Scenario 6: aside on marcus@slack; marcus@slack and marcus@discord merged; marcus@discord present.
    let marcus_slack = memory("person/marcus@slack");
    let marcus_discord = MemoryId::generate();
    let erin = MemoryId::generate();
    let merged: HashMap<MemoryId, MemoryId> = [
        (marcus_slack.id, marcus_slack.id),
        (marcus_discord, marcus_slack.id),
    ]
    .into();
    let class_of = |id| Ok(*merged.get(&id).unwrap_or(&id));

    let aside = entry(Teller::Participant(erin), Visibility::PrivateToTeller);
    // The present discord stub shares Marcus's class, so the subject-guard suppresses the aside.
    assert!(!visible(&aside, &marcus_slack, &[erin, marcus_discord], &class_of).unwrap());
}

#[test]
fn unmerged_stubs_do_not_suppress() {
    // Scenario 7: two distinct Marcus stubs, unmerged — a different present stub is a different
    // entity, so the subject-guard does not fire (the named cost of operator-only merging).
    let marcus_slack = memory("person/marcus@slack");
    let marcus_discord = MemoryId::generate();
    let erin = MemoryId::generate();
    let aside = entry(Teller::Participant(erin), Visibility::PrivateToTeller);
    assert!(visible(&aside, &marcus_slack, &[erin, marcus_discord], &identity).unwrap());
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

// --- Link visibility tests ---

fn link_vis(
    from: MemoryId,
    to: MemoryId,
    told_by: Option<Teller>,
    visibility: Visibility,
) -> LinkVis {
    LinkVis {
        from,
        to,
        visibility,
        told_by,
        told_in: None,
    }
}

#[test]
fn link_public_is_always_visible() {
    let (erin, marcus) = (MemoryId::generate(), MemoryId::generate());
    let link = link_vis(
        erin,
        marcus,
        Some(Teller::Participant(erin)),
        Visibility::Public,
    );
    assert!(link_visible(&link, false, &[marcus], &identity).unwrap());
    assert!(link_visible(&link, false, &[], &identity).unwrap());
    assert!(link_visible(&link, false, &[erin, marcus], &identity).unwrap());
}

#[test]
fn link_attributed_is_always_visible() {
    let (erin, marcus) = (MemoryId::generate(), MemoryId::generate());
    let link = link_vis(
        erin,
        marcus,
        Some(Teller::Participant(erin)),
        Visibility::Attributed,
    );
    assert!(link_visible(&link, false, &[marcus], &identity).unwrap());
    assert!(link_visible(&link, false, &[], &identity).unwrap());
}

#[test]
fn link_private_hidden_when_teller_absent() {
    let (erin, marcus) = (MemoryId::generate(), MemoryId::generate());
    let link = link_vis(
        erin,
        marcus,
        Some(Teller::Participant(erin)),
        Visibility::PrivateToTeller,
    );
    // Teller absent, no one present: hidden.
    assert!(!link_visible(&link, false, &[], &identity).unwrap());
    // Teller absent, a stranger present: hidden.
    let stranger = MemoryId::generate();
    assert!(!link_visible(&link, false, &[stranger], &identity).unwrap());
}

#[test]
fn link_private_visible_when_teller_present_and_target_absent() {
    let (erin, marcus) = (MemoryId::generate(), MemoryId::generate());
    let link = link_vis(
        erin,
        marcus,
        Some(Teller::Participant(erin)),
        Visibility::PrivateToTeller,
    );
    // Teller present, target absent: visible.
    assert!(link_visible(&link, false, &[erin], &identity).unwrap());
}

#[test]
fn link_private_hidden_when_target_present() {
    let (erin, marcus) = (MemoryId::generate(), MemoryId::generate());
    let link = link_vis(
        erin,
        marcus,
        Some(Teller::Participant(erin)),
        Visibility::PrivateToTeller,
    );
    // Teller and target both present: hidden by the subject-guard.
    assert!(!link_visible(&link, false, &[erin, marcus], &identity).unwrap());
}

#[test]
fn link_private_visible_when_teller_is_the_target() {
    // A self-link: teller is both endpoints. The subject-guard does not fire for the teller.
    let erin = MemoryId::generate();
    let marcus = MemoryId::generate();
    let link = link_vis(
        erin,
        erin,
        Some(Teller::Participant(erin)),
        Visibility::PrivateToTeller,
    );
    // Teller present (as target): visible.
    assert!(link_visible(&link, false, &[erin], &identity).unwrap());
    // Teller and a stranger present: visible (stranger is not a subject).
    assert!(link_visible(&link, false, &[erin, marcus], &identity).unwrap());
}

#[test]
fn link_private_symmetric_hidden_when_either_endpoint_present() {
    let (erin, marcus) = (MemoryId::generate(), MemoryId::generate());
    let link = link_vis(
        erin,
        marcus,
        Some(Teller::Participant(erin)),
        Visibility::PrivateToTeller,
    );
    // Symmetric: both endpoints are subjects. Marcus present (teller absent): hidden.
    assert!(!link_visible(&link, true, &[marcus], &identity).unwrap());
    // Teller and Marcus both present: hidden (Marcus is a subject, not the teller).
    assert!(!link_visible(&link, true, &[erin, marcus], &identity).unwrap());
}

#[test]
fn link_private_symmetric_visible_when_teller_present_other_absent() {
    let (erin, marcus) = (MemoryId::generate(), MemoryId::generate());
    let link = link_vis(
        erin,
        marcus,
        Some(Teller::Participant(erin)),
        Visibility::PrivateToTeller,
    );
    // Teller present, other endpoint absent: visible (teller is a subject but doesn't block).
    assert!(link_visible(&link, true, &[erin], &identity).unwrap());
}

#[test]
fn link_exclude_hidden_when_excludee_present() {
    let (erin, marcus, dave) = (
        MemoryId::generate(),
        MemoryId::generate(),
        MemoryId::generate(),
    );
    let link = link_vis(
        erin,
        marcus,
        Some(Teller::Participant(erin)),
        Visibility::Exclude(vec![dave]),
    );
    // Teller present, excludee present: hidden.
    assert!(!link_visible(&link, false, &[erin, dave], &identity).unwrap());
    // Teller present, excludee absent: visible.
    assert!(link_visible(&link, false, &[erin], &identity).unwrap());
}

#[test]
fn link_exclude_hidden_when_target_present() {
    let (erin, marcus, dave) = (
        MemoryId::generate(),
        MemoryId::generate(),
        MemoryId::generate(),
    );
    let link = link_vis(
        erin,
        marcus,
        Some(Teller::Participant(erin)),
        Visibility::Exclude(vec![dave]),
    );
    // Teller and target present: hidden by the subject-guard.
    assert!(!link_visible(&link, false, &[erin, marcus], &identity).unwrap());
}

#[test]
fn link_subject_guard_is_class_aware() {
    // A private link erin → marcus@slack; marcus@slack and marcus@discord merged; marcus@discord present.
    let (erin, marcus_slack, marcus_discord) = (
        MemoryId::generate(),
        MemoryId::generate(),
        MemoryId::generate(),
    );
    let merged: HashMap<MemoryId, MemoryId> =
        [(marcus_slack, marcus_slack), (marcus_discord, marcus_slack)].into();
    let class_of = |id| Ok(*merged.get(&id).unwrap_or(&id));
    let link = link_vis(
        erin,
        marcus_slack,
        Some(Teller::Participant(erin)),
        Visibility::PrivateToTeller,
    );
    // The present discord stub shares Marcus's class, so the subject-guard suppresses the link.
    assert!(!link_visible(&link, false, &[erin, marcus_discord], &class_of).unwrap());
}

#[test]
fn link_marker_picks_the_register_by_posture() {
    let general = MarkerRoom {
        name: room_display("context/general"),
        confidential: false,
    };
    // Public carries no marker; attributed the lighter provenance register; a confidence the
    // teller-private register.
    assert_eq!(
        link_marker(&Visibility::Public, "Erin", Some(&general)),
        None
    );
    assert_eq!(
        link_marker(&Visibility::Attributed, "Erin", Some(&general)),
        Some("[via Erin in #general]".to_owned())
    );
    assert_eq!(
        link_marker(&Visibility::Attributed, "Erin", None),
        Some("[via Erin]".to_owned())
    );
    assert_eq!(
        link_marker(&Visibility::PrivateToTeller, "Erin", None),
        Some("[teller-private, told by Erin]".to_owned())
    );
}

#[test]
fn link_default_visibility_direct_belief_is_private() {
    // Erin (teller) → Marcus: a direct belief about someone else. PrivateToTeller.
    let (erin, marcus) = (MemoryId::generate(), MemoryId::generate());
    assert_eq!(
        default_link_visibility(
            erin,
            "person/erin",
            marcus,
            "person/marcus",
            &Teller::Participant(erin),
        ),
        Visibility::PrivateToTeller
    );
    // Also when the teller is the target (Marcus → Erin, told by Erin).
    assert_eq!(
        default_link_visibility(
            marcus,
            "person/marcus",
            erin,
            "person/erin",
            &Teller::Participant(erin),
        ),
        Visibility::PrivateToTeller
    );
}

#[test]
fn link_default_visibility_relayed_fact_is_attributed() {
    // Erin relays "Dave mentors Grace" — she is neither endpoint. Attributed.
    let (erin, dave, grace) = (
        MemoryId::generate(),
        MemoryId::generate(),
        MemoryId::generate(),
    );
    assert_eq!(
        default_link_visibility(
            dave,
            "person/dave",
            grace,
            "person/grace",
            &Teller::Participant(erin),
        ),
        Visibility::Attributed
    );
}

#[test]
fn link_default_visibility_self_link_is_public() {
    // A self-link (teller is both endpoints): Public.
    let erin = MemoryId::generate();
    assert_eq!(
        default_link_visibility(
            erin,
            "person/erin",
            erin,
            "person/erin",
            &Teller::Participant(erin),
        ),
        Visibility::Public
    );
}

#[test]
fn link_default_visibility_non_person_target_is_public() {
    // A link to a non-person target: Public.
    let (erin, project) = (MemoryId::generate(), MemoryId::generate());
    assert_eq!(
        default_link_visibility(
            erin,
            "person/erin",
            project,
            "project/hooli",
            &Teller::Participant(erin),
        ),
        Visibility::Public
    );
}

#[test]
fn link_explain_returns_correct_verdict() {
    let (erin, marcus) = (MemoryId::generate(), MemoryId::generate());
    let link = link_vis(
        erin,
        marcus,
        Some(Teller::Participant(erin)),
        Visibility::PrivateToTeller,
    );
    use super::VisibilityDecision;
    // Teller absent: TellerAbsent.
    assert_eq!(
        link_explain(&link, false, &[], &identity).unwrap(),
        VisibilityDecision::TellerAbsent
    );
    // Teller present, target absent: TellerPresent.
    assert_eq!(
        link_explain(&link, false, &[erin], &identity).unwrap(),
        VisibilityDecision::TellerPresent
    );
    // Target present: SubjectPresent.
    assert_eq!(
        link_explain(&link, false, &[erin, marcus], &identity).unwrap(),
        VisibilityDecision::SubjectPresent
    );
}
