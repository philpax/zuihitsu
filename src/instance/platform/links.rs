//! Participant and context link resolution and minting: asserting or retracting a connector-authored
//! structural edge, and resolving each endpoint to its memory id (minting on first contact where the
//! assert path allows it).

use crate::{
    event::{EventPayload, EventSource, LinkPosture, LinkSource, Visibility},
    graph::GraphError,
    ids::{MemoryId, PersonId},
    instance::{
        InstanceError,
        platform::{LinkError, LinkNode, Platform},
    },
    memory::identity::{resolve_context, resolve_or_mint_context, resolve_or_mint_participant},
    vocabulary::RelationName,
};

impl Platform<'_> {
    /// Assert (or, with `remove`, retract) a structural link a connector authored between two of its
    /// own scoped memories — a channel's or a participant's placement in a guild, say. Both endpoints
    /// are named under the connector's platform, so a connector can only ever link memories it owns.
    /// The edge is `Public` (a structural fact, not a told aside) and carries
    /// [`LinkSource::PlatformConnector`], so an audit reads which connector authored it. `same_as` is refused:
    /// cross-platform identity is operator-confirmed, never a connector's to assert.
    ///
    /// On assert, each endpoint is resolved or minted, so a link lands even on first sight of the guild
    /// or member. On retract, the endpoints are resolved without minting — an edge to a node that does
    /// not exist cannot exist, so the retract is a no-op rather than a pointless mint.
    pub fn link(
        &self,
        from: &LinkNode,
        to: &LinkNode,
        relation: &str,
        platform: &str,
        remove: bool,
    ) -> Result<(), LinkError> {
        let relation = RelationName::new(relation);
        if relation == RelationName::SameAs {
            return Err(LinkError::SameAsForbidden);
        }
        let engine = &self.server.engine;
        if engine.graph.lock().relation(relation.as_str())?.is_none() {
            return Err(LinkError::UnknownRelation(relation));
        }

        let endpoints = if remove {
            match (self.resolve_node(from)?, self.resolve_node(to)?) {
                (Some(from_id), Some(to_id)) => Some((from_id, to_id)),
                _ => None,
            }
        } else {
            // Each resolve mints and materializes its endpoint under one lock, so both classes are
            // already visible to the edge apply below.
            let from_id = self.resolve_or_mint_node(from)?;
            let to_id = self.resolve_or_mint_node(to)?;
            Some((from_id, to_id))
        };
        let Some((from_id, to_id)) = endpoints else {
            return Ok(());
        };

        let payload = if remove {
            EventPayload::link_removed(from_id, to_id, relation)
        } else {
            EventPayload::link_created(
                from_id,
                to_id,
                relation,
                LinkPosture {
                    source: LinkSource::PlatformConnector(platform.to_owned()),
                    // No teller and no told_in: a connector's structural edge has no human behind it,
                    // mirroring the operator-authored `same_as`.
                    told_by: None,
                    told_in: None,
                    visibility: Visibility::Public,
                },
            )
        };
        let now = engine.clock.now();
        engine.store.lock().append(
            now,
            EventSource::PlatformConnector(platform.to_owned()),
            vec![payload],
        )?;
        engine
            .graph
            .lock()
            .materialize_from(engine.store.lock().as_ref())?;
        Ok(())
    }

    /// Resolve a link endpoint to its memory id, minting one on first contact and leaving the mint
    /// materialized before returning — the assert path, where a guild or member seen for the first time
    /// should still take the edge.
    ///
    /// One graph guard spans the resolve, the mint's append, and the materialize (mirroring
    /// [`ensure_conversation`](Self::ensure_conversation)). Released between the append and the
    /// materialize, a concurrent first contact for the same identity would resolve against a graph that
    /// does not yet hold the mint and append a second `MemoryCreated` under the same qualified name;
    /// materializing that pair collides on the `memories.name` UNIQUE index and wedges every later fold.
    /// Graph before store, per the lock-ordering rule; the store is locked transiently within the span.
    pub(super) fn resolve_or_mint_node(&self, node: &LinkNode) -> Result<MemoryId, InstanceError> {
        let engine = &self.server.engine;
        let mut graph = engine.graph.lock();
        let head_before = engine.store.lock().head()?;
        let id = match node {
            LinkNode::Participant(person) => resolve_or_mint_participant(
                engine.store.lock().as_mut(),
                engine.clock.as_ref(),
                &graph,
                person.platform.as_str(),
                person.id.as_str(),
            )?,
            LinkNode::Context(locator) => resolve_or_mint_context(
                engine.store.lock().as_mut(),
                engine.clock.as_ref(),
                &graph,
                locator,
            )?,
        };
        // Materialize only when the resolver actually minted (the store head moved): a pure
        // resolve-hit folds nothing, so skipping keeps the guard span short and spares the in-memory
        // store's full-log read on hot paths like a roster loop.
        if engine.store.lock().head()? != head_before {
            graph.materialize_from(engine.store.lock().as_ref())?;
        }
        Ok(id)
    }

    /// Resolve a platform participant to their `person/*` stub, minting one on first contact and leaving
    /// the mint materialized before returning. The participant counterpart to
    /// [`resolve_or_mint_node`](Self::resolve_or_mint_node): the same single-guard atomicity, so two
    /// racing first-contact resolutions for the same identity cannot both miss and double-mint the stub.
    pub(super) fn resolve_or_mint_person(
        &self,
        person: &PersonId,
    ) -> Result<MemoryId, InstanceError> {
        let engine = &self.server.engine;
        let mut graph = engine.graph.lock();
        let head_before = engine.store.lock().head()?;
        let id = resolve_or_mint_participant(
            engine.store.lock().as_mut(),
            engine.clock.as_ref(),
            &graph,
            person.platform.as_str(),
            person.id.as_str(),
        )?;
        // Materialize only when the resolver actually minted (the store head moved); see
        // [`resolve_or_mint_node`](Self::resolve_or_mint_node).
        if engine.store.lock().head()? != head_before {
            graph.materialize_from(engine.store.lock().as_ref())?;
        }
        Ok(id)
    }

    /// Resolve a link endpoint to its memory id without minting — the retract path, where a missing
    /// endpoint means the edge never existed.
    fn resolve_node(&self, node: &LinkNode) -> Result<Option<MemoryId>, GraphError> {
        let graph = self.server.engine.graph.lock();
        match node {
            LinkNode::Participant(person) => {
                graph.participant_for(person.platform.as_str(), person.id.as_str())
            }
            LinkNode::Context(locator) => resolve_context(&graph, locator),
        }
    }
}
