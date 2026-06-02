//! The read-time visibility predicate (spec §Visibility).
//!
//! `visible(entry, memory, present_set)` decides whether a content entry may be surfaced to the
//! people currently present. It is applied identically to every live surface — brief composition and
//! search — so the agent never sees an entry it shouldn't through any channel. The hard case the
//! predicate exists for is the **subject-guard**: a private aside about someone never surfaces while
//! that someone is present, even though their teller is — something an access-control list can't
//! express, because in an ACL the subject would have read access to their own record.
//!
//! Presence is two-valued because identity is never inferred: a present participant either is a
//! confirmed member of an entity or is not. This cut resolves presence by **direct membership** in
//! the present set; Stage 7 upgrades [`is_present`] to traverse the `same_as` class, at which point
//! the class-aware scenarios (subject- and exclude-guards across merged stubs) become meaningful.

use crate::{
    event::{Teller, Visibility},
    graph::{EntryView, MemoryView},
    ids::MemoryId,
};

/// Whether `entry` (on `memory`) may surface to the participants in `present_set`.
pub fn visible(entry: &EntryView, memory: &MemoryView, present_set: &[MemoryId]) -> bool {
    let subject = subject_participant(memory);
    match &entry.visibility {
        Visibility::Public => true,
        Visibility::PrivateToTeller => {
            teller_present(&entry.told_by, present_set)
                && !subject_blocks(subject, &entry.told_by, present_set)
        }
        Visibility::Exclude(excluded) => {
            teller_present(&entry.told_by, present_set)
                && excluded.iter().all(|x| !is_present(*x, present_set))
                && !subject_blocks(subject, &entry.told_by, present_set)
        }
    }
}

/// The write-time default visibility (spec §Visibility → defaults). A participant relaying something
/// about *someone else* is private to that teller; self-disclosure, agent-authored content, and any
/// non-person memory default public. The `PrivateToTeller` default exists only to guard asides about
/// an absent person — it is not a general default.
pub fn default_visibility(memory: &MemoryView, teller: &Teller) -> Visibility {
    match (subject_participant(memory), teller) {
        (Some(subject), Teller::Participant(teller_id)) if *teller_id != subject => {
            Visibility::PrivateToTeller
        }
        _ => Visibility::Public,
    }
}

/// The inline marker a surviving teller-private entry carries when surfaced (spec §Visibility →
/// marker), so the model sees it as a flagged judgment call rather than neutral fact. The room
/// (`told_in`) and its confidentiality join the marker at Stage 8, when contexts exist.
pub fn teller_private_marker(teller: &str) -> String {
    format!("[teller-private, told by {teller}]")
}

/// The participant a memory is *about*: the identity of a `person/*` stub, or `None` for every other
/// namespace and for `self` (which therefore get no subject-guard). Stage 7 makes this the stub's
/// `same_as` class rather than the bare id.
fn subject_participant(memory: &MemoryView) -> Option<MemoryId> {
    memory
        .name
        .as_str()
        .starts_with("person/")
        .then_some(memory.id)
}

/// Whether `entity` is among those present. Direct membership for now; `same_as`-class-aware at
/// Stage 7.
fn is_present(entity: MemoryId, present_set: &[MemoryId]) -> bool {
    present_set.contains(&entity)
}

/// Whether the teller is present. The `agent` teller is defined as always present to itself;
/// `bootstrap` is never a present participant (its content is public, so this never gates it).
fn teller_present(teller: &Teller, present_set: &[MemoryId]) -> bool {
    match teller {
        Teller::Agent => true,
        Teller::Participant(id) => is_present(*id, present_set),
        Teller::Bootstrap => false,
    }
}

/// Whether a present subject should suppress this entry. Never for a non-person memory (no subject),
/// and never for self-disclosure (the subject is the teller); otherwise the subject being present
/// suppresses an aside about them.
fn subject_blocks(subject: Option<MemoryId>, teller: &Teller, present_set: &[MemoryId]) -> bool {
    let Some(subject) = subject else {
        return false;
    };
    if teller_is(subject, teller) {
        return false;
    }
    is_present(subject, present_set)
}

/// Whether `teller` is the participant `subject` (self-disclosure). Stage 7 makes this class-aware.
fn teller_is(subject: MemoryId, teller: &Teller) -> bool {
    matches!(teller, Teller::Participant(id) if *id == subject)
}
