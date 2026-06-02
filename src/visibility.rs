//! The read-time visibility predicate (spec §Visibility).
//!
//! `visible(entry, memory, present_set, class_of)` decides whether a content entry may be surfaced
//! to the people currently present. It is applied identically to every live surface — brief
//! composition and search — so the agent never sees an entry it shouldn't through any channel. The
//! hard case the predicate exists for is the **subject-guard**: a private aside about someone never
//! surfaces while that someone is present, even though their teller is — something an access-control
//! list can't express, because in an ACL the subject would have read access to their own record.
//!
//! Presence is two-valued because identity is never inferred: a present participant either is a
//! confirmed member of an entity or is not. Membership resolves over the `same_as` **class**, via the
//! injected `class_of` (a memory's class id, or itself when unmerged) — so a private aside about
//! `phil@slack` is suppressed when `phil@discord` is present once the operator has merged them.
//! Injecting the resolver keeps the predicate free of I/O (and trivially testable) while letting the
//! caller back it with the graph.

use crate::{
    event::{Teller, Visibility},
    graph::{EntryView, GraphError, MemoryView},
    ids::MemoryId,
};

/// Resolves a memory id to its `same_as`-class id (or itself when unmerged). Fallible because the
/// production resolver reads the graph; a leak-safe predicate must propagate that rather than guess.
pub type ClassOf<'a> = dyn Fn(MemoryId) -> Result<MemoryId, GraphError> + 'a;

/// Whether `entry` (on `memory`) may surface to the participants in `present_set`, resolving
/// identity through `class_of`.
pub fn visible(
    entry: &EntryView,
    memory: &MemoryView,
    present_set: &[MemoryId],
    class_of: &ClassOf,
) -> Result<bool, GraphError> {
    let subject = subject_participant(memory);
    Ok(match &entry.visibility {
        Visibility::Public => true,
        Visibility::PrivateToTeller => {
            teller_present(&entry.told_by, present_set, class_of)?
                && !subject_blocks(subject, &entry.told_by, present_set, class_of)?
        }
        Visibility::Exclude(excluded) => {
            teller_present(&entry.told_by, present_set, class_of)?
                && no_excludee_present(excluded, present_set, class_of)?
                && !subject_blocks(subject, &entry.told_by, present_set, class_of)?
        }
    })
}

/// The write-time default visibility (spec §Visibility → defaults). A participant relaying something
/// about *someone else* is private to that teller; self-disclosure, agent-authored content, and any
/// non-person memory default public. The `PrivateToTeller` default exists only to guard asides about
/// an absent person — it is not a general default. Identity here is the write-time stub, not the
/// class: a teller attributing to a specific stub of themselves is still self-disclosure.
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

/// The participant a memory is *about*: a `person/*` stub, or `None` for every other namespace and
/// for `self` (which therefore get no subject-guard). The bare stub id; the predicate resolves it to
/// its class through `class_of`.
fn subject_participant(memory: &MemoryView) -> Option<MemoryId> {
    memory
        .name
        .as_str()
        .starts_with("person/")
        .then_some(memory.id)
}

/// Whether `entity` is present — some member of its `same_as` class is in `present_set`.
fn is_present(
    entity: MemoryId,
    present_set: &[MemoryId],
    class_of: &ClassOf,
) -> Result<bool, GraphError> {
    let target = class_of(entity)?;
    for present in present_set {
        if class_of(*present)? == target {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Whether the teller is present. The `agent` teller is defined as always present to itself;
/// `bootstrap` is never a present participant (its content is public, so this never gates it).
fn teller_present(
    teller: &Teller,
    present_set: &[MemoryId],
    class_of: &ClassOf,
) -> Result<bool, GraphError> {
    match teller {
        Teller::Agent => Ok(true),
        Teller::Participant(id) => is_present(*id, present_set, class_of),
        Teller::Bootstrap => Ok(false),
    }
}

/// Whether a present subject should suppress this entry. Never for a non-person memory (no subject),
/// and never for self-disclosure (the subject's class is the teller's); otherwise the subject being
/// present suppresses an aside about them.
fn subject_blocks(
    subject: Option<MemoryId>,
    teller: &Teller,
    present_set: &[MemoryId],
    class_of: &ClassOf,
) -> Result<bool, GraphError> {
    let Some(subject) = subject else {
        return Ok(false);
    };
    if teller_is(subject, teller, class_of)? {
        return Ok(false);
    }
    is_present(subject, present_set, class_of)
}

/// Whether `teller` is the participant `subject` — same `same_as` class (self-disclosure).
fn teller_is(subject: MemoryId, teller: &Teller, class_of: &ClassOf) -> Result<bool, GraphError> {
    match teller {
        Teller::Participant(id) => Ok(class_of(*id)? == class_of(subject)?),
        _ => Ok(false),
    }
}

/// Whether any excluded party is present, resolving each over its class.
fn no_excludee_present(
    excluded: &[MemoryId],
    present_set: &[MemoryId],
    class_of: &ClassOf,
) -> Result<bool, GraphError> {
    for excludee in excluded {
        if is_present(*excludee, present_set, class_of)? {
            return Ok(false);
        }
    }
    Ok(true)
}
