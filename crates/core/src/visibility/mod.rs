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

use serde::{Deserialize, Serialize};

use crate::{
    event::{Teller, Visibility},
    graph::{EntryView, GraphError, LinkVis, MemoryView},
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
    Ok(match &entry.visibility {
        Visibility::Public => VisibilityDecision::Public,
        Visibility::Attributed => VisibilityDecision::Attributed,
        Visibility::PrivateToTeller => {
            if !teller_present(&entry.told_by, present_set, class_of)? {
                VisibilityDecision::TellerAbsent
            } else if subject_blocks(subject, &entry.told_by, present_set, class_of)? {
                VisibilityDecision::SubjectPresent
            } else {
                VisibilityDecision::TellerPresent
            }
        }
        Visibility::Exclude(excluded) => {
            if !teller_present(&entry.told_by, present_set, class_of)? {
                VisibilityDecision::TellerAbsent
            } else if !no_excludee_present(excluded, present_set, class_of)? {
                VisibilityDecision::ExcludeePresent
            } else if subject_blocks(subject, &entry.told_by, present_set, class_of)? {
                VisibilityDecision::SubjectPresent
            } else {
                VisibilityDecision::NotExcluded
            }
        }
    })
}

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
/// link has no text body. Takes the resolved `MarkerRoom` (from `told_in`) so a teller-private marker
/// can name the room and its confidentiality, mirroring content entries.
pub fn link_marker(
    visibility: &Visibility,
    teller: &str,
    room: Option<&MarkerRoom>,
) -> Option<String> {
    match visibility {
        Visibility::Public => None,
        Visibility::Attributed => Some(attributed_marker(teller, room)),
        Visibility::PrivateToTeller | Visibility::Exclude(_) => {
            Some(teller_private_marker(teller, room))
        }
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

/// The inline marker an entry carries when it surfaces, chosen by its posture (spec §Visibility →
/// provenance markers): none for `Public`, the lighter `[via …]` for `Attributed`, the teller-private
/// marker for a confidence. The one place a surface (search, the brief) turns a visible non-public
/// entry into its marker, so the two registers can never be applied to the wrong posture.
pub fn entry_marker(
    visibility: &Visibility,
    teller: &str,
    room: Option<&MarkerRoom>,
) -> Option<String> {
    match visibility {
        Visibility::Public => None,
        Visibility::Attributed => Some(attributed_marker(teller, room)),
        Visibility::PrivateToTeller | Visibility::Exclude(_) => {
            Some(teller_private_marker(teller, room))
        }
    }
}

/// The inline marker an `Attributed` entry carries when surfaced (spec §Visibility → provenance
/// markers): the lighter register, naming only the source without the language of confidence, since
/// the entry is visible to anyone and the marker is the whole signal — secondhand, weigh it as such.
/// `[via Erin]`, or `[via Erin in #general]` when the room is known.
pub fn attributed_marker(teller: &str, room: Option<&MarkerRoom>) -> String {
    match room {
        Some(MarkerRoom { name, .. }) => format!("[via {teller} in {name}]"),
        None => format!("[via {teller}]"),
    }
}

/// The marker display name of a [`Namespace::Context`] memory: its handle with the namespace
/// stripped and a `#` prefix (`context/leads` → `#leads`), the room reference the agent sees in a
/// teller-private marker.
pub fn room_display(context_name: &str) -> String {
    let subject = context_name
        .strip_prefix(Namespace::Context.prefix())
        .unwrap_or(context_name);
    format!("#{subject}")
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
mod tests;
