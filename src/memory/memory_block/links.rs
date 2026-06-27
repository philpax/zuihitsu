//! Link and relation operations: registering, creating, removing, and traversing links.

use std::collections::BTreeSet;

use crate::{
    event::{EventPayload, LinkSource, Teller},
    graph::RelationView,
    ids::MemoryId,
    vocabulary::RelationName,
};

use super::{
    Authority, LinkDirection, LinkRef, MemoryBlock, MemoryError, RelationSpec, parse_cardinality,
};

impl MemoryBlock {
    /// Links out of this memory's whole `same_as` class under `relation`, in the relation's canonical
    /// forward direction — `mem:outgoing("mentor_of")` is who the identity mentors. A traversing read
    /// (locks the class). The relation may be named by either label, but the *method* picks the
    /// direction, not the label: use [`MemoryBlock::incoming`] for the reverse. An unregistered relation
    /// is a teachable error. A symmetric relation has no direction, so `outgoing` and `incoming` return
    /// the same neighbours under it.
    pub fn outgoing(&mut self, id: MemoryId, relation: &str) -> Result<Vec<LinkRef>, MemoryError> {
        self.directed_links(id, relation, LinkDirection::Outgoing)
    }

    /// Links into this memory's whole `same_as` class under `relation` — `mem:incoming("mentor_of")`
    /// is who mentors the identity. The reverse of [`MemoryBlock::outgoing`]; see it for the details.
    pub fn incoming(&mut self, id: MemoryId, relation: &str) -> Result<Vec<LinkRef>, MemoryError> {
        self.directed_links(id, relation, LinkDirection::Incoming)
    }

    /// Every link out of this memory's whole `same_as` class, in every relation and both directions —
    /// `mem:links()`, the relationship overview. A traversing read (locks the class). Like the
    /// relation-registry reads, and unlike the content reads, this reflects only committed state: a
    /// link created or removed in this same block is not yet visible here.
    pub fn links(&mut self, id: MemoryId) -> Result<Vec<LinkRef>, MemoryError> {
        self.class_link_refs(id)
    }

    /// Link `from` to `to` under a registered relation (e.g. flag a thread `active_in` the context).
    pub fn link(
        &mut self,
        from: MemoryId,
        to: MemoryId,
        relation: RelationName,
    ) -> Result<(), MemoryError> {
        self.change_link(from, to, relation, true)
    }

    /// Remove such a link.
    pub fn unlink(
        &mut self,
        from: MemoryId,
        to: MemoryId,
        relation: RelationName,
    ) -> Result<(), MemoryError> {
        self.change_link(from, to, relation, false)
    }

    /// Propose that two stubs are the same human across platforms, for the adjudication pass to weigh
    /// (spec §Cross-platform identity → adjudicated merge). This is *not* a merge: it buffers an inert
    /// `MergeProposed` — no `same_as`, no class change, nothing surfaces across the would-be merge — so
    /// the agent records its judgment without itself collapsing two identities' visibility. A proposal
    /// naming one memory twice is rejected as a teachable error; everything else (whether the two are
    /// truly the same) is the adjudicator's call, on the evidence.
    pub fn propose_merge(&mut self, from: MemoryId, to: MemoryId) -> Result<(), MemoryError> {
        if from == to {
            return Err(MemoryError::MergeProposalInvalid);
        }
        self.touched.insert(from);
        self.touched.insert(to);
        self.buffer.push(EventPayload::merge_proposed(from, to));
        Ok(())
    }

    /// Register a link relation, accessible thereafter under either label; re-registering an existing
    /// name updates it in place (the materializer upserts). The cardinality strings are parsed here, at
    /// the block boundary, so a bad value is a teachable error rather than a silent mis-store.
    pub fn register_relation(&mut self, spec: RelationSpec) -> Result<(), MemoryError> {
        let from_card = parse_cardinality(&spec.from_card)?;
        let to_card = parse_cardinality(&spec.to_card)?;
        self.buffer.push(EventPayload::LinkTypeRegistered {
            name: RelationName::new(spec.name),
            inverse: RelationName::new(spec.inverse),
            from_card,
            to_card,
            symmetric: spec.symmetric,
            reflexive: spec.reflexive,
        });
        Ok(())
    }

    /// Every registered relation (committed), for `links.list`. A plain read of the projection; this
    /// block's pending registrations are not yet reflected, like every other committed read.
    pub fn all_relations(&self) -> Result<Vec<RelationView>, MemoryError> {
        Ok(self.engine.graph.lock().all_relations()?)
    }

    /// A single registered relation by either label (committed), for `links.get`, or `None`.
    pub fn relation(&self, name: &str) -> Result<Option<RelationView>, MemoryError> {
        Ok(self.engine.graph.lock().relation(name)?)
    }

    /// Shared body of [`MemoryBlock::outgoing`] and [`MemoryBlock::incoming`]: resolve the relation to
    /// its canonical label, then keep the class's links under it that run the wanted way (either way for
    /// a symmetric relation).
    fn directed_links(
        &mut self,
        id: MemoryId,
        relation: &str,
        want: LinkDirection,
    ) -> Result<Vec<LinkRef>, MemoryError> {
        let view = self
            .relation(relation)?
            .ok_or_else(|| MemoryError::UnknownRelation(RelationName::new(relation)))?;
        Ok(self
            .class_link_refs(id)?
            .into_iter()
            .filter(|link| link.relation == view.name && (view.symmetric || link.direction == want))
            .collect())
    }

    /// Every link from `id`'s `same_as` class to a memory *outside* the class, oriented against the
    /// class and carrying the far memory's name for legible rendering. The shared engine of the three
    /// link readers. Edges internal to the class — the `same_as` plumbing and any other within-identity
    /// edge — are dropped: a relationship the agent reasons about points out of the identity. Committed
    /// state only (see [`MemoryBlock::links`]). A traversing read, so it touches the whole class.
    fn class_link_refs(&mut self, id: MemoryId) -> Result<Vec<LinkRef>, MemoryError> {
        let (members, refs) = {
            let graph = self.engine.graph.lock();
            let members = graph.class_members(id)?;
            let class: BTreeSet<MemoryId> = members.iter().copied().collect();
            let mut refs = Vec::new();
            for edge in graph.class_links(id)? {
                let (direction, other_id) =
                    match (class.contains(&edge.from), class.contains(&edge.to)) {
                        (true, false) => (LinkDirection::Outgoing, edge.to),
                        (false, true) => (LinkDirection::Incoming, edge.from),
                        // Within-class (both ends in the identity) or unrelated: not a relationship.
                        _ => continue,
                    };
                let Some(other) = graph.memory_by_id(other_id)? else {
                    continue;
                };
                // Resolve the teller's label off the held guard (teller_label re-locks the graph and
                // would deadlock here); a participant teller is a committed person memory.
                let told_by = match &edge.told_by {
                    None => None,
                    Some(Teller::Agent) => Some("you".to_owned()),
                    Some(Teller::Bootstrap) => Some("genesis".to_owned()),
                    Some(Teller::Participant(teller_id)) => Some(
                        graph
                            .memory_by_id(*teller_id)?
                            .map(|memory| memory.name.as_str().to_owned())
                            .unwrap_or_else(|| "someone".to_owned()),
                    ),
                };
                refs.push(LinkRef {
                    relation: edge.relation,
                    other: other.id,
                    other_name: other.name,
                    direction,
                    source: edge.source,
                    told_by,
                });
            }
            (members, refs)
        };
        self.touch_class(id, members);
        Ok(refs)
    }

    /// Whether `relation` is registered under either label — checking this block's pending
    /// `LinkTypeRegistered`s (read-your-writes) before the committed registry, so a relation registered
    /// and linked within the same block is recognized (spec §Read-your-writes within a block).
    fn relation_registered(&self, relation: &RelationName) -> Result<bool, MemoryError> {
        let pending = self.buffer.iter().any(|event| {
            matches!(
                event,
                EventPayload::LinkTypeRegistered { name, inverse, .. }
                    if name == relation || inverse == relation
            )
        });
        if pending {
            return Ok(true);
        }
        Ok(self
            .engine
            .graph
            .lock()
            .relation(relation.as_str())?
            .is_some())
    }

    /// Enforce that `relation` is registered — the graph stores an unregistered relation as given, so
    /// the contract is checked here — then buffer the create/remove and touch both endpoints.
    fn change_link(
        &mut self,
        from: MemoryId,
        to: MemoryId,
        relation: RelationName,
        create: bool,
    ) -> Result<(), MemoryError> {
        if !self.relation_registered(&relation)? {
            return Err(MemoryError::UnknownRelation(relation));
        }
        // Cross-platform identity is operator-asserted only: a participant must not be able to steer
        // the agent into merging (or splitting) two identities, which would collapse their visibility
        // classes (spec §Cross-platform identity is operator-asserted only).
        if relation == RelationName::SameAs && self.authority == Authority::Platform {
            return Err(MemoryError::MergeForbidden);
        }
        // A link from or to `self` modifies the self model — barred outside the console.
        self.guard_self(from)?;
        self.guard_self(to)?;
        // Operator-authored links carry operator provenance; the agent's own carry `Agent`. (The
        // adjudicated `same_as` is authored by the merge-adjudication pass directly, not through a block,
        // so it never reaches this seam — see `LinkSource::Adjudicated`.)
        let source = match self.authority {
            Authority::Operator => LinkSource::Operator,
            Authority::Platform => LinkSource::Agent,
        };
        self.touched.insert(from);
        self.touched.insert(to);
        self.buffer.push(if create {
            EventPayload::LinkCreated {
                from,
                to,
                relation,
                source,
                // The relationship's provenance is the turn's teller, the same teller a content append
                // would carry — so a later read of a belief-bearing link knows who asserted it.
                told_by: Some(self.teller.clone()),
            }
        } else {
            EventPayload::link_removed(from, to, relation)
        });
        Ok(())
    }
}
