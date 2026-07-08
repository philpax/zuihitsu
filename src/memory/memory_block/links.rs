//! Link and relation operations: registering, creating, removing, and traversing links.

use std::collections::BTreeSet;

use crate::{
    event::{EventPayload, LinkSource, MergeProposalSource, Teller, Visibility},
    graph::{Graph, RelationView},
    ids::MemoryId,
    memory::visibility::link_visible,
    time::TemporalRef,
    vocabulary::RelationName,
};

use super::{
    Authority, LinkDirection, LinkOptions, LinkRef, MemoryBlock, MemoryError, RelationSpec,
    VisibilityChoice, parse_cardinality,
};

impl MemoryBlock {
    /// Links out of this memory's whole `same_as` class under `relation`, in the relation's canonical
    /// forward direction — `mem:outgoing("mentors")` is who the identity mentors. A traversing read
    /// (locks the class). The relation may be named by either label, but the *method* picks the
    /// direction, not the label: use [`MemoryBlock::incoming`] for the reverse. An unregistered relation
    /// is a teachable error. A symmetric relation has no direction, so `outgoing` and `incoming` return
    /// the same neighbours under it.
    pub fn outgoing(&mut self, id: MemoryId, relation: &str) -> Result<Vec<LinkRef>, MemoryError> {
        self.directed_links(id, relation, LinkDirection::Outgoing)
    }

    /// Links into this memory's whole `same_as` class under `relation` — `mem:incoming("mentors")`
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

    /// Link `from` to `to` under a registered relation (e.g. record that one person `knows` another).
    /// An optional `opts` table carries `visibility` to force the posture instead of the write-time
    /// default.
    pub fn link(
        &mut self,
        from: MemoryId,
        to: MemoryId,
        relation: RelationName,
        opts: Option<LinkOptions>,
    ) -> Result<(), MemoryError> {
        self.change_link(from, to, relation, true, opts.and_then(|o| o.visibility))
    }

    /// Remove such a link.
    pub fn unlink(
        &mut self,
        from: MemoryId,
        to: MemoryId,
        relation: RelationName,
    ) -> Result<(), MemoryError> {
        self.change_link(from, to, relation, false, None)
    }

    /// Propose that two stubs are the same human across platforms, for the adjudication pass to weigh
    /// (spec §Cross-platform identity → adjudicated merge). This is *not* a merge: it buffers an inert
    /// `MergeProposed` — no `same_as`, no class change, nothing surfaces across the would-be merge — so
    /// the agent records its judgment without itself collapsing two identities' visibility. `rationale`
    /// carries the proposer's stated grounds for the coincidence, if any, which the adjudicator weighs
    /// against the two stubs' independently-recorded facts. A proposal naming one memory twice is
    /// rejected as a teachable error; everything else (whether the two are truly the same) is the
    /// adjudicator's call, on the evidence.
    pub fn propose_merge(
        &mut self,
        from: MemoryId,
        to: MemoryId,
        rationale: Option<String>,
    ) -> Result<(), MemoryError> {
        if from == to {
            return Err(MemoryError::MergeProposalInvalid);
        }
        self.touched.insert(from);
        self.touched.insert(to);
        self.buffer.push(EventPayload::merge_proposed(
            from,
            to,
            MergeProposalSource::Agent,
            rationale,
        ));
        Ok(())
    }

    /// Register a link relation, accessible thereafter under either label; re-registering an existing
    /// name updates it in place (the materializer upserts). The cardinality strings are parsed here, at
    /// the block boundary, so a bad value is a teachable error rather than a silent mis-store.
    pub fn register_relation(&mut self, spec: RelationSpec) -> Result<(), MemoryError> {
        let from_card = parse_cardinality(&spec.from_card)?;
        let to_card = parse_cardinality(&spec.to_card)?;
        self.buffer.push(EventPayload::LinkTypeRegistered {
            name: RelationName::new(&spec.name),
            inverse: RelationName::new(&spec.inverse),
            from_card,
            to_card,
            symmetric: spec.symmetric,
            reflexive: spec.reflexive,
            description: spec.description,
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
    /// Visibility-filtered through `link_visible` when an audience is present, mirroring the content
    /// entry reads.
    fn class_link_refs(&mut self, id: MemoryId) -> Result<Vec<LinkRef>, MemoryError> {
        let (members, refs) = {
            let graph = self.engine.graph.lock();
            let members = graph.class_members(id)?;
            let class: BTreeSet<MemoryId> = members.iter().copied().collect();
            let class_of = |mid| graph.class_id(mid).map(|class| class.unwrap_or(mid));
            let audience = !self.present_set.is_empty();
            let mut refs = Vec::new();
            for edge in graph.class_links(id)? {
                let (direction, other_id) =
                    match (class.contains(&edge.from), class.contains(&edge.to)) {
                        (true, false) => (LinkDirection::Outgoing, edge.to),
                        (false, true) => (LinkDirection::Incoming, edge.from),
                        // Within-class (both ends in the identity) or unrelated: not a relationship.
                        _ => continue,
                    };
                // Visibility filter: when an audience is present, filter private links.
                if audience {
                    let symmetric = graph
                        .relation(edge.relation.as_str())?
                        .map(|r| r.symmetric)
                        .unwrap_or(false);
                    if !link_visible(&edge.link_vis(), symmetric, &self.present_set, &class_of)? {
                        continue;
                    }
                }
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
                // The far memory's freshest dated fact, so a link to a dated event carries *when*.
                // Committed state only and not visibility-filtered, mirroring the link read itself.
                let occurred_at = latest_dated_occurrence(&graph, other.id)?;
                refs.push(LinkRef {
                    relation: edge.relation,
                    other: other.id,
                    other_name: other.name,
                    direction,
                    source: edge.source,
                    told_by,
                    occurred_at,
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
        visibility: Option<VisibilityChoice>,
    ) -> Result<(), MemoryError> {
        if !self.relation_registered(&relation)? {
            return Err(MemoryError::UnknownRelation(relation));
        }
        // Cross-platform identity is operator-asserted only: a participant must not be able to steer
        // the agent into merging (or splitting) two identities, which would collapse their visibility
        // classes (spec §Cross-platform identity is operator-asserted only). The agent nonetheless reads
        // for `link("same_as", other)` as "these are the same person" — its stated intent is a merge —
        // so a create routes to the proposal path (an inert `MergeProposed` the adjudication pass weighs)
        // rather than crashing the block and rolling back its innocent sibling writes. A retraction stays
        // operator-only: the agent can neither assert nor undo a `same_as` directly from a turn.
        if relation == RelationName::SameAs && self.authority == Authority::Platform {
            if create {
                return self.propose_merge(from, to, None);
            }
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
        let (visibility, told_in) = if create {
            let from_name = self.resolve_name(from)?;
            let to_name = self.resolve_name(to)?;
            let vis = self.resolve_link_visibility(
                from,
                from_name.as_ref().map(|n| n.as_str()),
                to,
                to_name.as_ref().map(|n| n.as_str()),
                &self.teller,
                visibility,
            )?;
            // The link carries the turn's context as its told_in, mirroring content entries — so a
            // teller-private marker can name the room it was said in.
            let told_in = self.told_in;
            (vis, told_in)
        } else {
            (Visibility::Public, None)
        };
        self.touched.insert(from);
        self.touched.insert(to);
        self.buffer.push(if create {
            EventPayload::link_created(
                from,
                to,
                relation,
                source,
                Some(self.teller.clone()),
                told_in,
                visibility,
            )
        } else {
            EventPayload::link_removed(from, to, relation)
        });
        Ok(())
    }
}

/// The far memory's representative occurrence for a link read: the most recent dated entry's
/// `occurred_at` over its whole `same_as` class, preferring an authored occurrence over an extracted
/// one, or `None` when it holds no dated fact. An authored date is ground truth (the agent stamped it
/// at append); an extracted one is inference the turn-end temporal extraction resolved, which can
/// misfire (anaphora like "that weekend" resolved against the clock). So the freshest authored date
/// wins, and an extracted date carries onto the handle only when the class holds no authored date at
/// all — a guess never shadows a stated fact. Within each tier, entries compose in commit order, so
/// the last dated one wins — the freshest dated fact, which for a linked event (a shipped decision, a
/// scheduled meeting) is the *when* the agent relays. Not link-visibility-filtered: this reads the far
/// memory's entries to find *when* a linked event happened, which is not gated on the link's audience
/// posture — the link read itself already filtered the edge through `link_visible`.
fn latest_dated_occurrence(
    graph: &Graph,
    id: MemoryId,
) -> Result<Option<TemporalRef>, MemoryError> {
    let mut latest_authored = None;
    let mut latest_extracted = None;
    for entry in graph.class_entries(id)? {
        if entry.occurred_at.is_some() {
            if entry.occurred_authored {
                latest_authored = entry.occurred_at;
            } else {
                latest_extracted = entry.occurred_at;
            }
        }
    }
    Ok(latest_authored.or(latest_extracted))
}

#[cfg(test)]
mod tests {
    use super::latest_dated_occurrence;
    use crate::{
        event::{Event, EventPayload, Teller, Visibility},
        graph::Graph,
        ids::{EntryId, MemoryId, Namespace, Seq},
        time::{CivilDate, TemporalRef, Timestamp},
    };

    fn event(seq: u64, payload: EventPayload) -> Event {
        Event {
            seq: Seq(seq),
            recorded_at: Timestamp::from_millis(1),
            payload,
        }
    }

    #[test]
    fn a_link_targets_authored_date_outranks_a_newer_extracted_one() {
        // The neighborhood line's `[when …]` must carry the target's authored date over a newer
        // extracted one: an authored July cutover outranks a June date the extraction later resolved
        // for a sibling "that weekend" statement, so a recap relayed from the handle keeps the stated
        // when rather than a clock-anchored guess.
        let mut graph = Graph::open_in_memory().unwrap();
        let id = MemoryId::generate();
        let authored = EntryId::generate();
        let extracted = EntryId::generate();
        let july = TemporalRef::Day(CivilDate("2026-07-20".into()));
        let june = TemporalRef::Day(CivilDate("2026-06-08".into()));
        let append = |seq, entry, occurred_at, text: &str| {
            event(
                seq,
                EventPayload::MemoryContentAppended {
                    id,
                    entry_id: entry,
                    asserted_at: Timestamp::from_millis(1),
                    occurred_at,
                    text: text.to_owned(),
                    told_by: Teller::Agent,
                    told_in: None,
                    visibility: Visibility::Public,
                },
            )
        };
        graph
            .apply(&event(
                1,
                EventPayload::memory_created(id, Namespace::Event.with_name("cutover")),
            ))
            .unwrap();
        graph
            .apply(&append(2, authored, Some(july.clone()), "cut billing over"))
            .unwrap();
        graph
            .apply(&append(
                3,
                extracted,
                None,
                "Devin owns the rollback that weekend",
            ))
            .unwrap();
        graph
            .apply(&event(
                4,
                EventPayload::entry_temporal_resolved(id, extracted, june.clone(), None),
            ))
            .unwrap();

        // Authored wins even though the extracted entry is newer.
        assert_eq!(
            latest_dated_occurrence(&graph, id).unwrap().as_ref(),
            Some(&july),
        );

        // With no authored date in the class, the extracted one still surfaces (the fallback).
        let mut graph = Graph::open_in_memory().unwrap();
        let only = MemoryId::generate();
        let entry = EntryId::generate();
        graph
            .apply(&event(
                1,
                EventPayload::memory_created(only, Namespace::Event.with_name("call")),
            ))
            .unwrap();
        graph
            .apply(&event(
                2,
                EventPayload::MemoryContentAppended {
                    id: only,
                    entry_id: entry,
                    asserted_at: Timestamp::from_millis(1),
                    occurred_at: None,
                    text: "that weekend".to_owned(),
                    told_by: Teller::Agent,
                    told_in: None,
                    visibility: Visibility::Public,
                },
            ))
            .unwrap();
        graph
            .apply(&event(
                3,
                EventPayload::entry_temporal_resolved(only, entry, june.clone(), None),
            ))
            .unwrap();
        assert_eq!(
            latest_dated_occurrence(&graph, only).unwrap().as_ref(),
            Some(&june),
        );
    }
}
