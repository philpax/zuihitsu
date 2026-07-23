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
//! `marcus@slack` is suppressed when `marcus@discord` is present once the operator has merged them.
//! Injecting the resolver keeps the predicate free of I/O (and trivially testable) while letting the
//! caller back it with the graph.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::{
    event::{Teller, Visibility},
    graph::{AttestationView, EntryView, GraphError, MemoryView},
    ids::{MemoryId, MemoryName, Namespace},
};

/// Resolves a memory id to its `same_as`-class id (or itself when unmerged). Fallible because the
/// production resolver reads the graph; a leak-safe predicate must propagate that rather than guess.
pub type ClassOf<'a> = dyn Fn(MemoryId) -> Result<MemoryId, GraphError> + 'a;

/// Why an entry is or isn't visible: the [`visible`] predicate's verdict with its reason. The three
/// visible verdicts and the four hidden ones are told apart by [`VisibilityDecision::is_visible`].
/// Carried so the console's brief trace can show not just whether a fact surfaced but why.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum VisibilityDecision {
    /// Surfaces to anyone — a public entry.
    Public,
    /// Surfaces to anyone — an attributed (secondhand-but-ordinary) entry. Visible like `Public`; it
    /// is told apart so a surface can attach the provenance marker and exclude it from descriptions.
    Attributed,
    /// Surfaces: teller-private, the teller is present, and the subject-guard does not block.
    TellerPresent,
    /// Surfaces: an `Exclude` entry with the teller present, no named excludee present, and no guard.
    NotExcluded,
    /// Hidden: a newer entry superseded this one.
    Superseded,
    /// Hidden: the teller is not present.
    TellerAbsent,
    /// Hidden by the subject-guard — the subject of this memory is present.
    SubjectPresent,
    /// Hidden: a named excludee is present.
    ExcludeePresent,
}

impl VisibilityDecision {
    /// Whether this verdict surfaces the entry.
    pub fn is_visible(self) -> bool {
        matches!(
            self,
            VisibilityDecision::Public
                | VisibilityDecision::Attributed
                | VisibilityDecision::TellerPresent
                | VisibilityDecision::NotExcluded
        )
    }
}

/// Whether `entry` (on `memory`) may surface to the participants in `present_set`, resolving
/// identity through `class_of`.
pub fn visible(
    entry: &EntryView,
    memory: &MemoryView,
    present_set: &[MemoryId],
    class_of: &ClassOf,
) -> Result<bool, GraphError> {
    Ok(explain(entry, memory, present_set, class_of)?.is_visible())
}

/// As [`visible`], but reporting *why* — the verdict the brief trace renders. A superseded entry is
/// never live on any surface (spec §Visibility → superseded entries are not live); the live entry
/// reads already exclude these in SQL, so this guard covers the search path, which resolves a vector
/// hit through `entry_by_id` (which does not filter) before this predicate.
///
/// An entry carries a *set* of attestations — each a (teller, posture) endorsement of its fact — and
/// the verdict is the **widest passing verdict** over the live ones: the fact surfaces if any teller
/// standing behind it clears the audience, and it surfaces under that teller's widest posture (a
/// `Public` founding attestation renders the fact even when a further teller's endorsement is a hidden
/// confidence). When none passes, the founding attestation's own failure verdict is returned, so a
/// singleton entry — the founding attestation alone, derived from `told_by`/`visibility` — is
/// bit-identical to reasoning over the entry's own fields, the make-or-break compatibility property.
pub fn explain(
    entry: &EntryView,
    memory: &MemoryView,
    present_set: &[MemoryId],
    class_of: &ClassOf,
) -> Result<VisibilityDecision, GraphError> {
    if entry.superseded_by.is_some() {
        return Ok(VisibilityDecision::Superseded);
    }
    let subject = subject_participant(memory.name.as_str(), memory.id);
    // The effective attestation set: the populated one, or the entry's founding attestation
    // synthesized from its own fields when a hand-built view left the set empty (the two agree for a
    // singleton, so a view constructed without the batched fetch reads exactly as it did before).
    let synthesized;
    let attestations: &[AttestationView] = if entry.attestations.is_empty() {
        synthesized = [founding_attestation(entry)];
        &synthesized
    } else {
        &entry.attestations
    };
    let mut widest: Option<VisibilityDecision> = None;
    // The founding attestation is first; its verdict is the failure returned when nothing passes.
    let mut founding_verdict = VisibilityDecision::TellerAbsent;
    for (index, attestation) in attestations.iter().enumerate() {
        let verdict = explain_attestation(attestation, subject, present_set, class_of)?;
        if index == 0 {
            founding_verdict = verdict;
        }
        // A withdrawn attestation never widens the verdict: only the history read carries such rows
        // (so the console can render the withdrawal). The founding row still supplies the failure
        // fallback above — the reason describes the founding posture either way.
        if attestation.retracted_reason.is_some() {
            continue;
        }
        if verdict.is_visible() {
            widest = Some(match widest {
                Some(current) => wider(current, verdict),
                None => verdict,
            });
        }
    }
    Ok(widest.unwrap_or(founding_verdict))
}

/// One attestation's verdict against the present set — today's per-posture logic parameterized by the
/// attestation's own teller and posture. The entry-level [`explain`] combines these across the
/// attestation set; the chip rule's [`visible_attestations`] filters by each one's own verdict.
fn explain_attestation(
    attestation: &AttestationView,
    subject: Option<MemoryId>,
    present_set: &[MemoryId],
    class_of: &ClassOf,
) -> Result<VisibilityDecision, GraphError> {
    let teller = &attestation.teller;
    Ok(match &attestation.posture {
        Visibility::Public => VisibilityDecision::Public,
        Visibility::Attributed => VisibilityDecision::Attributed,
        Visibility::PrivateToTeller => {
            if !teller_present(teller, present_set, class_of)? {
                VisibilityDecision::TellerAbsent
            } else if subject_blocks(subject, teller, present_set, class_of)? {
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
            } else if subject_blocks(subject, teller, present_set, class_of)? {
                VisibilityDecision::SubjectPresent
            } else {
                VisibilityDecision::NotExcluded
            }
        }
    })
}

/// The visible subset of an entry's attestations for the present audience — the chip rule's engine.
/// Each attestation passes on its own posture and teller, so a hidden attestation (a confidence whose
/// teller is absent, say, endorsing an otherwise-public fact) is absent from this set with no residue,
/// even though the fact itself renders. Renderers name the attesters from this subset; a superseded
/// entry yields none.
pub fn visible_attestations<'a>(
    entry: &'a EntryView,
    memory: &MemoryView,
    present_set: &[MemoryId],
    class_of: &ClassOf,
) -> Result<Vec<&'a AttestationView>, GraphError> {
    if entry.superseded_by.is_some() {
        return Ok(Vec::new());
    }
    let subject = subject_participant(memory.name.as_str(), memory.id);
    let mut visible = Vec::new();
    for attestation in &entry.attestations {
        // A withdrawn attestation never renders as a chip; the history read carries such rows only
        // so the console can show the withdrawal itself.
        if attestation.retracted_reason.is_some() {
            continue;
        }
        if explain_attestation(attestation, subject, present_set, class_of)?.is_visible() {
            visible.push(attestation);
        }
    }
    Ok(visible)
}

/// The founding attestation of an entry, synthesized from its own `told_by`/`told_in`/`asserted_at`/
/// `visibility` — the singleton set a view built without the batched attestation fetch reads as.
fn founding_attestation(entry: &EntryView) -> AttestationView {
    AttestationView::founding(
        entry.told_by.clone(),
        entry.told_in.clone(),
        entry.asserted_at,
        entry.visibility.clone(),
    )
}

/// The wider of two *visible* verdicts, so the entry surfaces under the most permissive posture any
/// teller standing behind it warrants: `Public` over `Attributed` over the teller-gated verdicts. Both
/// arguments are already visible (the caller only combines passing verdicts), so a non-visible verdict
/// ranks lowest and never wins.
fn wider(a: VisibilityDecision, b: VisibilityDecision) -> VisibilityDecision {
    if width_rank(a) >= width_rank(b) { a } else { b }
}

/// The audience breadth of a visible verdict, ordered widest first. Only the visible verdicts rank;
/// the hidden ones share the floor, since [`wider`] is only ever handed passing verdicts.
fn width_rank(verdict: VisibilityDecision) -> u8 {
    match verdict {
        VisibilityDecision::Public => 4,
        VisibilityDecision::Attributed => 3,
        VisibilityDecision::TellerPresent => 2,
        VisibilityDecision::NotExcluded => 1,
        _ => 0,
    }
}

/// The write-time default visibility (spec §Visibility → defaults). A participant relaying something
/// about *someone else* is private to that teller; self-disclosure and any non-person memory default
/// public. The `PrivateToTeller` default exists only to guard asides about an absent person — it is
/// not a general default. Identity here is the write-time stub, not the class: a teller attributing
/// to a specific stub of themselves is still self-disclosure. Agent-authored content *about a person*
/// has no default at all — it is required to classify itself before reaching here (see
/// the main crate's `memory_block`), since a re-recorded confidence silently defaulting public is a leak.
pub fn default_visibility(memory: &MemoryView, teller: &Teller) -> Visibility {
    default_visibility_named(memory.name.as_str(), memory.id, teller)
}

/// As [`default_visibility`], computed from a memory's name and id directly. The write path needs
/// this because an append's target may be a memory created earlier in the same block — present in
/// the block's buffer, not yet a full [`MemoryView`] from the graph.
pub fn default_visibility_named(name: &str, id: MemoryId, teller: &Teller) -> Visibility {
    match (subject_participant(name, id), teller) {
        (Some(subject), Teller::Participant(teller_id)) if *teller_id != subject => {
            Visibility::PrivateToTeller
        }
        _ => Visibility::Public,
    }
}

/// Whether the teller is present. The `agent` teller is defined as always present to itself;
/// `bootstrap` is never a present participant (its content is public, so this never gates it).
pub(super) fn teller_present(
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
pub(super) fn subject_blocks(
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
/// Whether any excluded party is present, resolving each over its class.
pub(super) fn no_excludee_present(
    excluded: &BTreeSet<MemoryId>,
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

/// Whether `teller` is the participant `subject` — same `same_as` class (self-disclosure).
pub(super) fn teller_is(
    subject: MemoryId,
    teller: &Teller,
    class_of: &ClassOf,
) -> Result<bool, GraphError> {
    match teller {
        Teller::Participant(id) => Ok(class_of(*id)? == class_of(subject)?),
        _ => Ok(false),
    }
}
/// Whether `entity` is present — some member of its `same_as` class is in `present_set`.
pub(super) fn is_present(
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

/// The participant a memory is *about*: a [`Namespace::Person`] stub, or `None` for every other
/// namespace and for `self` (which therefore get no subject-guard). The bare stub id; the
/// predicate resolves it to
/// its class through `class_of`. Public so the write path can ask "does this memory have a subject?"
/// — the case where an agent-authored entry has no protective default (see the main crate's `memory_block`).
pub fn subject_participant(name: &str, id: MemoryId) -> Option<MemoryId> {
    let is_person =
        MemoryName::new(name).namespaced().map(|n| n.namespace) == Ok(Namespace::Person);
    is_person.then_some(id)
}

/// Whether a wake-up on `entry`/`memory` is *for* someone present (spec §Agent-initiated speech). Its
/// target is the memory's subject (a [`Namespace::Person`] stub) together with the entry's teller when a
/// participant; an item with no such target — agent-authored on a non-person memory — targets no one
/// and is never delivered. Class-aware, like the predicate.
pub fn targets_present(
    entry: &EntryView,
    memory: &MemoryView,
    present_set: &[MemoryId],
    class_of: &ClassOf,
) -> Result<bool, GraphError> {
    if let Some(subject) = subject_participant(memory.name.as_str(), memory.id)
        && is_present(subject, present_set, class_of)?
    {
        return Ok(true);
    }
    if let Teller::Participant(teller) = &entry.told_by
        && is_present(*teller, present_set, class_of)?
    {
        return Ok(true);
    }
    Ok(false)
}

mod links;
mod markers;
#[cfg(test)]
mod tests;

pub use links::*;
pub use markers::*;
