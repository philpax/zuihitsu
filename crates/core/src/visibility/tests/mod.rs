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

mod links;
mod markers;
mod predicate;

pub(super) fn memory(name: &str) -> MemoryView {
    MemoryView {
        id: MemoryId::generate(),
        name: MemoryName::new(name),
        description: String::new(),
        volatility: Volatility::Medium,
        created_at: Timestamp::from_millis(0),
        tags: Vec::new(),
    }
}

pub(super) fn entry(told_by: Teller, visibility: Visibility) -> EntryView {
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
pub(super) fn identity(id: MemoryId) -> Result<MemoryId, GraphError> {
    Ok(id)
}

/// A resolved visible attestation for the marker-assembler tests: posture, teller name, whether it is
/// the agent, and no room/turn (the multi-teller forms drop those).
pub(super) fn marker_att(
    posture: Visibility,
    teller: &str,
    is_agent: bool,
) -> super::MarkerAttestation {
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

// --- Link visibility tests ---

pub(super) fn link_vis(
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
