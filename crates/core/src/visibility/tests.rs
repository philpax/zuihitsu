//! Visibility predicate tests (spec appendix scenarios 1, 3, 4, 5, 6, 7–10, 16). Asserts directly
//! on `visible(...)` and `default_visibility(...)` over hand-built memories, entries, present
//! sets, and a `class_of` resolver — deterministic and model-free.
use std::collections::{BTreeSet, HashMap};

use super::{
    MarkerRoom, MarkerTurn, default_link_visibility, default_visibility, link_explain, link_marker,
    link_visible, room_display, teller_private_marker, visible,
};
use crate::{
    event::{Teller, Visibility, Volatility},
    graph::{EntryOrigin, EntryView, GraphError, LinkVis, MemoryView},
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
        retracted_reason: None,
        origin: EntryOrigin::Recorded,
        // Left empty so the predicate reads the founding attestation off `told_by`/`visibility` —
        // the fallback that keeps a hand-built singleton bit-identical to the pre-attestation fold.
        attestations: Vec::new(),
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
    let general = MarkerTurn {
        turn_id: None,
        room: Some(MarkerRoom {
            name: room_display("context/general"),
            confidential: false,
        }),
    };
    assert_eq!(
        teller_private_marker("Erin", Some(&general)),
        "[teller-private, told by Erin in #general]"
    );
    // A #confidential room says so — the cross-context signal the agent reasons over (scenario 13).
    let leads = MarkerTurn {
        turn_id: None,
        room: Some(MarkerRoom {
            name: room_display("context/leads"),
            confidential: true,
        }),
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

/// A resolved visible attestation for the marker-assembler tests: posture, teller name, whether it is
/// the agent, and no room/turn (the multi-teller forms drop those).
fn marker_att(posture: Visibility, teller: &str, is_agent: bool) -> super::MarkerAttestation {
    super::MarkerAttestation {
        posture,
        teller: teller.to_owned(),
        is_agent,
        marker: MarkerTurn {
            turn_id: None,
            room: None,
        },
    }
}

#[test]
fn attestation_marker_names_multiple_attributed_tellers() {
    use super::entry_attestation_marker;
    // A lone attributed teller keeps the full single-teller register (room and turn token when known).
    let general = MarkerTurn {
        turn_id: None,
        room: Some(MarkerRoom {
            name: room_display("context/general"),
            confidential: false,
        }),
    };
    let lone = super::MarkerAttestation {
        posture: Visibility::Attributed,
        teller: "Erin".to_owned(),
        is_agent: false,
        marker: general,
    };
    assert_eq!(
        entry_attestation_marker(std::slice::from_ref(&lone)),
        Some("[via Erin in #general]".to_owned())
    );
    // Two attributed tellers are both named; a third and beyond fold into a count.
    assert_eq!(
        entry_attestation_marker(&[
            marker_att(Visibility::Attributed, "Marcus", false),
            marker_att(Visibility::Attributed, "Erin", false),
        ]),
        Some("[via Marcus, Erin]".to_owned())
    );
    assert_eq!(
        entry_attestation_marker(&[
            marker_att(Visibility::Attributed, "Marcus", false),
            marker_att(Visibility::Attributed, "Erin", false),
            marker_att(Visibility::Attributed, "Dave", false),
            marker_att(Visibility::Attributed, "Priya", false),
        ]),
        Some("[via Marcus, Erin, +2]".to_owned())
    );
}

#[test]
fn attestation_marker_skips_the_agent_in_a_consolidation_replacement() {
    use super::entry_attestation_marker;
    // A consolidation replacement is founded by the agent at `Attributed`, carrying the real tellers
    // as further attestations: the via-list draws from those, the agent skipped.
    assert_eq!(
        entry_attestation_marker(&[
            marker_att(Visibility::Attributed, "the agent", true),
            marker_att(Visibility::Attributed, "Erin", false),
            marker_att(Visibility::Attributed, "Dave", false),
        ]),
        Some("[via Erin, Dave]".to_owned())
    );
    // When only the agent remains visible, no via-marker renders (`[via the agent]` is never shown).
    assert_eq!(
        entry_attestation_marker(&[marker_att(Visibility::Attributed, "the agent", true)]),
        None
    );
}

#[test]
fn attestation_marker_corroborates_a_public_entry() {
    use super::entry_attestation_marker;
    // A public entry standing on its founding source alone carries no marker.
    assert_eq!(
        entry_attestation_marker(&[marker_att(Visibility::Public, "Erin", false)]),
        None
    );
    // Extra visible tellers ride an "also told by" corroboration marker: names for one or two.
    assert_eq!(
        entry_attestation_marker(&[
            marker_att(Visibility::Public, "Erin", false),
            marker_att(Visibility::Public, "Dave", false),
        ]),
        Some("[also told by Dave]".to_owned())
    );
    // Three or more corroborators fold into a count.
    assert_eq!(
        entry_attestation_marker(&[
            marker_att(Visibility::Public, "Erin", false),
            marker_att(Visibility::Public, "Dave", false),
            marker_att(Visibility::Public, "Priya", false),
            marker_att(Visibility::Attributed, "Marcus", false),
        ]),
        Some("[also told by 3 others]".to_owned())
    );
}

#[test]
fn attestation_marker_keeps_the_confidence_register() {
    use super::entry_attestation_marker;
    // A confidence surfaced to its teller keeps today's teller-private marker, unchanged.
    assert_eq!(
        entry_attestation_marker(&[marker_att(Visibility::PrivateToTeller, "Erin", false)]),
        Some("[teller-private, told by Erin]".to_owned())
    );
    // An empty visible set — a superseded or wholly-hidden entry — carries nothing.
    assert_eq!(entry_attestation_marker(&[]), None);
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
