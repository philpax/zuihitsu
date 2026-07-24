//! Link-level visibility: whether an edge shows to an audience, the write-time default a link
//! takes, and its provenance marker. The same posture logic as content entries, applied to edges —
//! which have no text body, so the marker rides the relationship line.

use crate::{
    event::{Teller, Visibility},
    graph::{GraphError, LinkVis},
    ids::MemoryId,
    visibility::{
        ClassOf, MarkerTurn, VisibilityDecision, attributed_marker, is_present,
        no_excludee_present, subject_participant, teller_is, teller_present, teller_private_marker,
    },
};

/// Whether `link` may surface to the participants in `present_set`, resolving identity through
/// `class_of`. The subject-guard protects the link's target: a `PrivateToTeller` link `A → B` is
/// hidden when B is present, mirroring a `PrivateToTeller` content entry on `person/b` hidden when B
/// is present. For a symmetric relation, both endpoints are subjects — the link is hidden when *either*
/// is present (unless that endpoint is the teller).
pub fn link_visible(
    link: &LinkVis,
    symmetric: bool,
    present_set: &[MemoryId],
    class_of: &ClassOf,
) -> Result<bool, GraphError> {
    Ok(link_explain(link, symmetric, present_set, class_of)?.is_visible())
}

/// As [`link_visible`], but reporting *why* — the verdict the brief trace renders. The teller-presence
/// and excludee checks are identical to the content predicate; the subject-guard differs: a directed
/// link's subject is its target (B for `A → B`), while a symmetric link's subjects are both endpoints.
pub fn link_explain(
    link: &LinkVis,
    symmetric: bool,
    present_set: &[MemoryId],
    class_of: &ClassOf,
) -> Result<VisibilityDecision, GraphError> {
    let teller = link.told_by.as_ref().unwrap_or(&Teller::Agent);
    Ok(match &link.visibility {
        Visibility::Public => VisibilityDecision::Public,
        Visibility::Attributed => VisibilityDecision::Attributed,
        Visibility::PrivateToTeller => {
            if !teller_present(teller, present_set, class_of)? {
                VisibilityDecision::TellerAbsent
            } else if link_subject_blocks(
                link.from,
                link.to,
                symmetric,
                teller,
                present_set,
                class_of,
            )? {
                VisibilityDecision::SubjectPresent
            } else {
                VisibilityDecision::TellerPresent
            }
        }
        Visibility::Exclude(excluded) => {
            if !teller_present(teller, present_set, class_of)? {
                VisibilityDecision::TellerAbsent
            } else if !no_excludee_present(excluded, present_set, class_of)? {
                VisibilityDecision::ExcludeePresent
            } else if link_subject_blocks(
                link.from,
                link.to,
                symmetric,
                teller,
                present_set,
                class_of,
            )? {
                VisibilityDecision::SubjectPresent
            } else {
                VisibilityDecision::NotExcluded
            }
        }
    })
}

/// Whether a present subject should suppress this link. For a directed link, the subject is the
/// target (B for `A → B`); for a symmetric link, both endpoints are subjects. A subject that is the
/// teller does not block (self-link or self-asserted). For a symmetric link, a subject present that is
/// *not* the teller blocks — the link is hidden when either endpoint is present unless that endpoint
/// is the teller.
fn link_subject_blocks(
    from: MemoryId,
    to: MemoryId,
    symmetric: bool,
    teller: &Teller,
    present_set: &[MemoryId],
    class_of: &ClassOf,
) -> Result<bool, GraphError> {
    // For a directed link, only the target (to) is a subject.
    // For a symmetric link, both endpoints are subjects.
    let subjects: Vec<MemoryId> = if symmetric { vec![from, to] } else { vec![to] };
    for subject in subjects {
        if teller_is(subject, teller, class_of)? {
            continue;
        }
        if is_present(subject, present_set, class_of)? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// The write-time default visibility for a link (spec §Visibility → defaults). A participant-asserted
/// belief link where the teller is one endpoint and the target is a *different* person defaults
/// `PrivateToTeller` — "I hate B" is a direct private belief about B. A link where the teller is
/// *neither* endpoint (a relayed fact: "Dave mentors Grace," told by Erin) defaults `Attributed` — it
/// is an ordinary secondhand fact, visible to anyone but carrying provenance. Self-links (teller is
/// both endpoints) and non-person targets default `Public`. Agent-authored links about a person must
/// classify explicitly — no default, the same teachable-error gate as content.
pub fn default_link_visibility(
    from: MemoryId,
    from_name: &str,
    to: MemoryId,
    to_name: &str,
    teller: &Teller,
) -> Visibility {
    let from_subject = subject_participant(from_name, from);
    let to_subject = subject_participant(to_name, to);
    // Self-links (teller is both endpoints) default Public — not a belief about someone else.
    if from == to {
        return Visibility::Public;
    }
    match (from_subject, to_subject, teller) {
        // Teller is one endpoint, the other is a different person: a direct belief about someone
        // else. PrivateToTeller.
        (Some(from_id), Some(to_id), Teller::Participant(teller_id))
            if *teller_id == from_id && *teller_id != to_id =>
        {
            Visibility::PrivateToTeller
        }
        (Some(from_id), Some(to_id), Teller::Participant(teller_id))
            if *teller_id == to_id && *teller_id != from_id =>
        {
            Visibility::PrivateToTeller
        }
        // Teller is neither endpoint: a relayed fact about two other people. Attributed — visible
        // to anyone, carrying provenance.
        (Some(_), Some(_), Teller::Participant(_)) => Visibility::Attributed,
        // Self-links, non-person targets, or non-participant tellers: Public.
        _ => Visibility::Public,
    }
}

/// The inline marker a surviving non-public link carries when surfaced, chosen by its posture. Same
/// registers as content entries: none for `Public`, `[via …]` for `Attributed`, the teller-private
/// marker for a confidence. Appended to the relationship line (`relation: handle [marker]`) since a
/// link has no text body. Takes the resolved `MarkerTurn` (from `told_in`) so a teller-private marker
/// can name the room and its confidentiality and embed a turn token, mirroring content entries.
pub fn link_marker(
    visibility: &Visibility,
    teller: &str,
    marker: Option<&MarkerTurn>,
) -> Option<String> {
    match visibility {
        Visibility::Public => None,
        Visibility::Attributed => Some(attributed_marker(teller, marker)),
        Visibility::PrivateToTeller | Visibility::Exclude(_) => {
            Some(teller_private_marker(teller, marker))
        }
    }
}
