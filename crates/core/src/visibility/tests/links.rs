//! Link-level visibility: edge postures, endpoint guards, defaults, and the link marker.

use crate::visibility::tests::*;

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
        Visibility::Exclude(BTreeSet::from([dave])),
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
        Visibility::Exclude(BTreeSet::from([dave])),
    );
    // Teller and target present: hidden by the subject-guard.
    assert!(!link_visible(&link, false, &[erin, marcus], &identity).unwrap());
}

#[test]
fn link_subject_guard_is_class_aware() {
    // A private link erin → marcus@forum; marcus@forum and marcus@chat merged; marcus@chat present.
    let (erin, marcus_forum, marcus_chat) = (
        MemoryId::generate(),
        MemoryId::generate(),
        MemoryId::generate(),
    );
    let merged: HashMap<MemoryId, MemoryId> =
        [(marcus_forum, marcus_forum), (marcus_chat, marcus_forum)].into();
    let class_of = |id| Ok(*merged.get(&id).unwrap_or(&id));
    let link = link_vis(
        erin,
        marcus_forum,
        Some(Teller::Participant(erin)),
        Visibility::PrivateToTeller,
    );
    // The present chat stub shares Marcus's class, so the subject-guard suppresses the link.
    assert!(!link_visible(&link, false, &[erin, marcus_chat], &class_of).unwrap());
}

#[test]
fn link_marker_picks_the_register_by_posture() {
    let general = MarkerTurn {
        turn_id: None,
        room: Some(MarkerRoom {
            name: room_display("context/general"),
            confidential: false,
        }),
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
    use crate::visibility::VisibilityDecision;
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
