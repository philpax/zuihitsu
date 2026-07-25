//! The mechanical link re-home step of the canonicalize pass.
//!
//! Write-time canonicalization routes a new link onto its endpoints' class primaries, but that only
//! helps classes that exist at write time: a link accrues on a stub, and only later does a merge form
//! the `same_as` class the stub belongs to, leaving the edge stranded on a non-primary member. This
//! step is the standing repair — purely mechanical, no model call. For every `same_as` class with more
//! than one member, each link attached to a non-primary member (that is neither `same_as` plumbing nor
//! a connector-maintained structural edge) is moved onto the class primary, its posture carried over
//! verbatim so the move re-asserts nothing.

use std::collections::HashSet;

use crate::{
    InstanceError,
    engine::Engine,
    event::{LinkPosture, LinkSource},
    graph::{Graph, GraphError},
    ids::MemoryId,
    memory::memory_block::{MemoryBlock, RehomedLink},
    vocabulary::RelationName,
};

/// Re-home every link scattered across a `same_as` class's non-primary members onto the class primary,
/// buffering the moves into `block`. Reads the committed graph, so a class formed by this same sweep's
/// mint-and-bind is not yet visible here and is repaired on the next sweep — the step is a cursorless,
/// idempotent catch-up, cheap once the classes are clean.
pub(super) fn rehome_scattered_links(
    engine: &Engine,
    block: &mut MemoryBlock,
) -> Result<(), InstanceError> {
    for link in plan_rehomes(engine)? {
        block.rehome_link(link);
    }
    Ok(())
}

/// Plan the re-homes over the committed graph in one locked read: gather every edge touching a
/// multi-member class, canonicalize both endpoints to their class primaries, and split the ones that
/// move from the parallel copies a primary edge already covers.
fn plan_rehomes(engine: &Engine) -> Result<Vec<RehomedLink>, InstanceError> {
    let graph = engine.graph.lock();

    // Gather every edge touching a multi-member class, deduped by raw endpoints and relation: an edge
    // between two merged identities is returned by both classes' `class_links`, and re-homing it twice
    // would emit duplicate events.
    let mut seen = HashSet::new();
    let mut edges = Vec::new();
    for primary in graph.same_as_classes()? {
        for edge in graph.class_links(primary)? {
            if seen.insert((edge.from, edge.to, edge.relation.clone())) {
                edges.push(edge);
            }
        }
    }

    let class_of =
        |id: MemoryId| -> Result<MemoryId, GraphError> { Ok(graph.class_id(id)?.unwrap_or(id)) };
    // A symmetric edge is unordered, so a survivor check must match either endpoint order — normalize
    // the key to the lower id first.
    let norm = |a: MemoryId, b: MemoryId, rel: &RelationName, symmetric: bool| {
        if symmetric && b < a {
            (b, a, rel.clone())
        } else {
            (a, b, rel.clone())
        }
    };

    // Seed the parallel-edge set with every edge already sitting at canonical endpoints (both ends
    // primaries), so a stub edge re-homing onto one is recognised as a copy the primary already carries.
    let mut present: HashSet<(MemoryId, MemoryId, RelationName)> = HashSet::new();
    for edge in &edges {
        let cf = class_of(edge.from)?;
        let ct = class_of(edge.to)?;
        if cf == edge.from && ct == edge.to && cf != ct {
            let symmetric = symmetric_of(&graph, &edge.relation)?;
            present.insert(norm(cf, ct, &edge.relation, symmetric));
        }
    }

    let mut plans = Vec::new();
    for edge in edges {
        // `same_as` is the class structure itself, member-level by definition; a connector maintains its
        // structural edges on the exact stub it owns. Neither is re-homed.
        if edge.relation == RelationName::SameAs
            || matches!(edge.source, LinkSource::PlatformConnector(_))
        {
            continue;
        }
        let cf = class_of(edge.from)?;
        let ct = class_of(edge.to)?;
        if cf == edge.from && ct == edge.to {
            continue; // Already canonical: nothing to move.
        }
        if cf == ct {
            // Both endpoints canonicalize into one identity: a within-class edge whose re-home would be
            // a self-loop. The link reads already drop within-class edges, so leave the stored row be.
            continue;
        }
        let symmetric = symmetric_of(&graph, &edge.relation)?;
        // The first mover to a canonical slot creates it; a later member carrying the same relation to
        // the same far identity is a survivor drop — the primary's edge wins the parallel.
        let survivor = !present.insert(norm(cf, ct, &edge.relation, symmetric));
        plans.push(RehomedLink {
            stored_from: edge.from,
            stored_to: edge.to,
            canonical_from: cf,
            canonical_to: ct,
            relation: edge.relation,
            posture: LinkPosture {
                source: edge.source,
                told_by: edge.told_by,
                told_in: edge.told_in,
                visibility: edge.visibility,
            },
            survivor,
        });
    }
    Ok(plans)
}

/// Whether `relation` is symmetric (unregistered or unknown reads as directed).
fn symmetric_of(graph: &Graph, relation: &RelationName) -> Result<bool, GraphError> {
    Ok(graph
        .relation(relation.as_str())?
        .map(|view| view.symmetric)
        .unwrap_or(false))
}
