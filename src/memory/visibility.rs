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
    // A superseded entry is never live, on any surface (spec §Visibility → superseded entries are
    // not live). The live entry reads already exclude these in SQL; this guards the search path,
    // which resolves a vector hit through `entry_by_id` (which does not filter) before this predicate.
    if entry.superseded_by.is_some() {
        return Ok(false);
    }
    let subject = subject_participant(memory.name.as_str(), memory.id);
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
/// about *someone else* is private to that teller; self-disclosure and any non-person memory default
/// public. The `PrivateToTeller` default exists only to guard asides about an absent person — it is
/// not a general default. Identity here is the write-time stub, not the class: a teller attributing
/// to a specific stub of themselves is still self-disclosure. Agent-authored content *about a person*
/// has no default at all — it is required to classify itself before reaching here (see
/// [`crate::memory::memory_block`]), since a re-recorded confidence silently defaulting public is a leak.
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

/// The room a teller-private entry was told in, resolved for the marker: its display name (e.g.
/// `#leads`) and whether it is `#confidential`. The caller resolves an entry's `told_in` to this at
/// build time (see [`room_display`]), keeping this module I/O-free, mirroring the `class_of`
/// injection pattern.
pub struct MarkerRoom {
    pub name: String,
    pub confidential: bool,
}

/// The inline marker a surviving teller-private entry carries when surfaced (spec §Visibility →
/// marker), so the model sees it as a flagged judgment call rather than neutral fact. It names the
/// teller, and — when the entry's `told_in` room is known — the room and, if the room is
/// `#confidential`, that it was said in confidence: `[teller-private, told by Erin in #leads
/// (confidential)]`. The marker is baked into `recent_facts` at brief-build time, so a later
/// cross-context surfacing can be recognized as one.
pub fn teller_private_marker(teller: &str, room: Option<&MarkerRoom>) -> String {
    match room {
        Some(MarkerRoom {
            name,
            confidential: true,
        }) => format!("[teller-private, told by {teller} in {name} (confidential)]"),
        Some(MarkerRoom {
            name,
            confidential: false,
        }) => format!("[teller-private, told by {teller} in {name}]"),
        None => format!("[teller-private, told by {teller}]"),
    }
}

/// The marker display name of a `context/*` memory: its handle with the namespace stripped and a `#`
/// prefix (`context/leads` → `#leads`), the room reference the agent sees in a teller-private marker.
pub fn room_display(context_name: &str) -> String {
    format!(
        "#{}",
        context_name
            .strip_prefix("context/")
            .unwrap_or(context_name)
    )
}

/// The participant a memory is *about*: a `person/*` stub, or `None` for every other namespace and
/// for `self` (which therefore get no subject-guard). The bare stub id; the predicate resolves it to
/// its class through `class_of`. Public so the write path can ask "does this memory have a subject?"
/// — the case where an agent-authored entry has no protective default (see [`crate::memory::memory_block`]).
pub fn subject_participant(name: &str, id: MemoryId) -> Option<MemoryId> {
    name.starts_with("person/").then_some(id)
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

/// Whether a wake-up on `entry`/`memory` is *for* someone present (spec §Agent-initiated speech). Its
/// target is the memory's subject (a `person/*` stub) together with the entry's teller when a
/// participant; an item with no such target — agent-authored on a non-person memory — targets no one
/// and is never delivered. Class-aware, like the predicate.
pub(crate) fn targets_present(
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

#[cfg(test)]
mod tests {
    //! Visibility predicate tests (spec appendix scenarios 1, 3, 4, 5, 6, 7–10, 16). Asserts directly
    //! on `visible(...)` and `default_visibility(...)` over hand-built memories, entries, present
    //! sets, and a `class_of` resolver — deterministic and model-free.
    use std::collections::HashMap;

    use super::{MarkerRoom, default_visibility, room_display, teller_private_marker, visible};
    use crate::{
        event::{Teller, Visibility, Volatility},
        graph::{EntryView, GraphError, MemoryView},
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
}
