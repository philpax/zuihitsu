//! Provenance-marker assembly: the `[via …]`, `[also told by …]`, and teller-private registers a
//! rendered entry or link carries, built from the visible attestation subset the predicate module
//! resolves. Pure string assembly — the I/O (teller names, rooms) stays with the caller, mirroring
//! the `class_of` injection.

use crate::{
    event::Visibility,
    ids::{Namespace, TurnId},
};

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
