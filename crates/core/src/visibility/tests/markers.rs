//! Marker assembly: the register choices and the attestation chip lists.

use super::*;

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
fn entry_marker_picks_the_register_by_posture() {
    use crate::visibility::entry_marker;
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

#[test]
fn attestation_marker_names_multiple_attributed_tellers() {
    use crate::visibility::entry_attestation_marker;
    // A lone attributed teller keeps the full single-teller register (room and turn token when known).
    let general = MarkerTurn {
        turn_id: None,
        room: Some(MarkerRoom {
            name: room_display("context/general"),
            confidential: false,
        }),
    };
    let lone = crate::visibility::MarkerAttestation {
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
    use crate::visibility::entry_attestation_marker;
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
    use crate::visibility::entry_attestation_marker;
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
    use crate::visibility::entry_attestation_marker;
    // A confidence surfaced to its teller keeps today's teller-private marker, unchanged.
    assert_eq!(
        entry_attestation_marker(&[marker_att(Visibility::PrivateToTeller, "Erin", false)]),
        Some("[teller-private, told by Erin]".to_owned())
    );
    // An empty visible set — a superseded or wholly-hidden entry — carries nothing.
    assert_eq!(entry_attestation_marker(&[]), None);
}
