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
    graph::{AttestationView, EntryView, GraphError, LinkVis, MemoryView},
    ids::{MemoryId, MemoryName, Namespace, TurnId},
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

/// A resolved visibility marker: the turn the fact was asserted (for cross-linking) and the room it
/// was told in (for display). Wraps [`MarkerRoom`] so the marker functions can embed a
/// `[turn:<ulid>]` token alongside the room reference. Built from a `ConversationRef` by the caller
/// (search, brief composition) via [`Graph::marker_ref`].
pub struct MarkerTurn {
    /// The turn the fact was asserted, when `told_in` is `Some(ConversationRef::Turn(...))`.
    pub turn_id: Option<TurnId>,
    /// The resolved room, when `told_in` is `Some(ConversationRef::Room(...))`.
    pub room: Option<MarkerRoom>,
}

/// The inline marker a surviving teller-private entry carries when surfaced (spec §Visibility →
/// marker), so the model sees it as a flagged judgment call rather than neutral fact. It names the
/// teller, and — when the entry's `told_in` room is known — the room and, if the room is
/// `#confidential`, that it was said in confidence: `[teller-private, told by Erin in #leads
/// (confidential) [turn:01KX…]]`. When a turn id is known, a `[turn:<ulid>]` token is embedded so
/// a renderer can resolve the reference into a link. The marker is baked into
/// `recent_facts` at brief-build time, so a later cross-context surfacing can be recognized as one.
pub fn teller_private_marker(teller: &str, marker: Option<&MarkerTurn>) -> String {
    let room = marker.and_then(|m| m.room.as_ref());
    let turn = marker.and_then(|m| m.turn_id.as_ref());
    let body = match room {
        Some(MarkerRoom {
            name,
            confidential: true,
        }) => format!("told by {teller} in {name} (confidential)"),
        Some(MarkerRoom {
            name,
            confidential: false,
        }) => format!("told by {teller} in {name}"),
        None => format!("told by {teller}"),
    };
    format_turn(&format!("[teller-private, {body}]"), turn)
}

/// The inline marker an entry carries when it surfaces, chosen by its posture (spec §Visibility →
/// provenance markers): none for `Public`, the lighter `[via …]` for `Attributed`, the teller-private
/// marker for a confidence. The one place a surface (search, the brief) turns a visible non-public
/// entry into its marker, so the two registers can never be applied to the wrong posture.
pub fn entry_marker(
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

/// The inline marker an `Attributed` entry carries when surfaced (spec §Visibility → provenance
/// markers): the lighter register, naming only the source without the language of confidence, since
/// the entry is visible to anyone and the marker is the whole signal — secondhand, weigh it as such.
/// `[via Erin [turn:01KX…]]`, or `[via Erin in #general [turn:01KX…]]` when the room is known.
pub fn attributed_marker(teller: &str, marker: Option<&MarkerTurn>) -> String {
    let room = marker.and_then(|m| m.room.as_ref());
    let turn = marker.and_then(|m| m.turn_id.as_ref());
    let body = match room {
        Some(MarkerRoom { name, .. }) => format!("via {teller} in {name}"),
        None => format!("via {teller}"),
    };
    format_turn(&format!("[{body}]"), turn)
}

/// One visible attestation resolved for marker rendering: its own audience posture, its teller's
/// display name, whether that teller is the agent (the synthesizer of a consolidation replacement,
/// never named in a via- or corroboration-list), and its resolved conversation reference. The caller
/// (search, brief) resolves each attestation of [`visible_attestations`] into one of these — the I/O
/// (teller name, room) stays out of the pure predicate module, mirroring the `class_of` injection.
pub struct MarkerAttestation {
    pub posture: Visibility,
    pub teller: String,
    pub is_agent: bool,
    pub marker: MarkerTurn,
}

/// The inline provenance marker an entry carries, built from its **visible** attestation subset for
/// the present audience (the chip rule — [`visible_attestations`] supplies the subset, so a hidden
/// attestation is already absent and leaves no residue here). The register is the widest visible
/// posture:
///
/// - **Public**: freely shareable, so the founding source goes unmarked; any further visible tellers
///   ride an `[also told by …]` corroboration marker (names for one or two, a count beyond).
/// - **Attributed**: a `[via …]` marker naming the visible attesting tellers — the agent skipped, so
///   a consolidation replacement founded [`Teller::Agent`] draws its via-list from the real tellers it
///   carried, and renders no marker when only the agent remains visible. A lone teller keeps the full
///   [`attributed_marker`] (room and turn token); several are named terse, a count beyond two.
/// - **Confidence** (`PrivateToTeller`/`Exclude`): today's [`teller_private_marker`] over the widest
///   visible confidence, unchanged.
///
/// `None` when the widest visible posture warrants no marker — a plain public entry, or an
/// agent-only attributed one. The attestations are founding-first, matching [`visible_attestations`].
pub fn entry_attestation_marker(visible: &[MarkerAttestation]) -> Option<String> {
    let widest = visible.iter().map(|a| posture_rank(&a.posture)).max()?;
    match widest {
        3 => also_told_marker(visible),
        2 => via_marker(visible),
        _ => {
            let confidence = visible.iter().find(|a| posture_rank(&a.posture) == 1)?;
            Some(teller_private_marker(
                &confidence.teller,
                Some(&confidence.marker),
            ))
        }
    }
}

/// The audience breadth of an attestation's own posture, ordered widest first: `Public` over
/// `Attributed` over the teller-gated confidences. Drives which register [`entry_attestation_marker`]
/// renders in.
fn posture_rank(posture: &Visibility) -> u8 {
    match posture {
        Visibility::Public => 3,
        Visibility::Attributed => 2,
        Visibility::PrivateToTeller | Visibility::Exclude(_) => 1,
    }
}

/// The `[via …]` marker for an attributed entry, naming the visible attesting tellers with the agent
/// skipped. A lone teller keeps the full [`attributed_marker`] (room and turn token); two are named;
/// beyond two the tail is a count, keeping the marker terse under the brief's budget. `None` when only
/// the agent attests (the consolidation-replacement case — the synthesizer is not a source).
fn via_marker(visible: &[MarkerAttestation]) -> Option<String> {
    let attesters: Vec<&MarkerAttestation> = visible
        .iter()
        .filter(|a| matches!(a.posture, Visibility::Attributed) && !a.is_agent)
        .collect();
    match attesters.as_slice() {
        [] => None,
        [only] => Some(attributed_marker(&only.teller, Some(&only.marker))),
        [a, b] => Some(format!("[via {}, {}]", a.teller, b.teller)),
        [a, b, rest @ ..] => Some(format!("[via {}, {}, +{}]", a.teller, b.teller, rest.len())),
    }
}

/// The `[also told by …]` corroboration marker for a public entry: the further visible tellers beyond
/// the founding source (itself unmarked, since public content is freely shareable), the agent and any
/// confidence skipped. Names for one or two corroborators, a count beyond. `None` when the public
/// entry stands on its founding source alone.
fn also_told_marker(visible: &[MarkerAttestation]) -> Option<String> {
    let corroborators: Vec<&MarkerAttestation> = visible
        .iter()
        .skip(1)
        .filter(|a| posture_rank(&a.posture) >= 2 && !a.is_agent)
        .collect();
    match corroborators.as_slice() {
        [] => None,
        [only] => Some(format!("[also told by {}]", only.teller)),
        [a, b] => Some(format!("[also told by {}, {}]", a.teller, b.teller)),
        more => Some(format!("[also told by {} others]", more.len())),
    }
}

/// Append a `[turn:<ulid>]` token inside the closing bracket of a marker string, when a turn id is
/// known. The token is the canonical reference form, so a renderer can resolve it into a link.
/// Without a turn id, the marker is returned unchanged.
fn format_turn(marker: &str, turn: Option<&TurnId>) -> String {
    match turn {
        Some(id) => {
            let turn_token = format!("[turn:{}]", id.0);
            // Insert before the closing bracket.
            let close = marker.rfind(']').unwrap_or(marker.len());
            let (head, tail) = marker.split_at(close);
            // `tail` starts at `]`; insert the turn token before it.
            format!("{head} {turn_token}{tail}")
        }
        None => marker.to_owned(),
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

#[cfg(test)]
mod tests;
