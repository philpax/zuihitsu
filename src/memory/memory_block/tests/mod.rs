//! Shared fixtures and the concern-grouped test submodules for the memory block.

mod authority;
mod conflict_suggestions;
mod consolidation;
mod content_limit;
mod free_merge;
mod mutation_guards;
mod writes;

pub(super) use super::{AppendOptions, Authority, MemoryBlock, MemoryError, VisibilityChoice};
use crate::{
    clock::ManualClock,
    engine::Engine,
    event::{Cardinality, EventPayload, EventSource, LinkPosture, LinkSource, Teller, Visibility},
    graph::Graph,
    ids::{ConversationId, MemoryId, MemoryName, Namespace},
    model::embed::Embedder,
    store::{MemoryStore, Store},
    time::Timestamp,
    vector::VectorIndex,
    vocabulary::RelationName,
};

use std::sync::Arc;

/// A block over an empty in-memory graph and a conversation with no context — enough to exercise
/// the write invariants directly, no Lua VM and no store materialization involved. The engine's
/// store is a throwaway: these tests read `into_effects` and never commit.
pub(super) fn block(
    graph: Graph,
    clock: ManualClock,
    teller: Teller,
    authority: Authority,
) -> MemoryBlock {
    let engine = Engine::new(Box::new(MemoryStore::new()), graph, Box::new(clock));
    MemoryBlock::new(
        engine,
        teller,
        authority,
        Some(ConversationId::generate()),
        None,
        Vec::new(),
        TEST_MAX_ENTRY_CHARS,
    )
    .unwrap()
}

/// A block like [`block`] but with no conversation attached, so its links carry `told_in: None`. That
/// keeps a link's provenance a deterministic function of its inputs across two separately-built blocks —
/// what the redundant-link-guard test leans on to re-derive the exact posture a first block committed.
pub(super) fn block_without_conversation(
    graph: Graph,
    clock: ManualClock,
    teller: Teller,
    authority: Authority,
) -> MemoryBlock {
    let engine = Engine::new(Box::new(MemoryStore::new()), graph, Box::new(clock));
    MemoryBlock::new(
        engine,
        teller,
        authority,
        None,
        None,
        Vec::new(),
        TEST_MAX_ENTRY_CHARS,
    )
    .unwrap()
}

/// A block with semantic retrieval attached — the embedder and vector index the dedup check needs.
/// The caller owns the `InMemoryVectorIndex` so it can seed vectors before the block's dedup check
/// runs.
pub(super) fn block_with_retrieval(
    graph: Graph,
    clock: ManualClock,
    teller: Teller,
    authority: Authority,
    embedder: Arc<dyn Embedder>,
    vectors: Box<dyn VectorIndex>,
) -> MemoryBlock {
    let engine = Engine::with_retrieval(
        Box::new(MemoryStore::new()),
        graph,
        Box::new(clock),
        embedder,
        vectors,
    );
    MemoryBlock::new(
        engine,
        teller,
        authority,
        Some(ConversationId::generate()),
        None,
        Vec::new(),
        TEST_MAX_ENTRY_CHARS,
    )
    .unwrap()
}

/// The character limit the test block enforces — generous enough that existing test content passes,
/// while still exercising the limit in the dedicated oversized-content tests.
const TEST_MAX_ENTRY_CHARS: usize = 10_000;

/// A graph seeded with the `self` memory and the `created_by` and `same_as` relations — the
/// minimum to exercise the self-write and merge guards, which key on the resolved `self` id and on
/// the relation. Returns the graph and `self`'s id.
pub(super) fn graph_with_self() -> (Graph, MemoryId) {
    let mut store = MemoryStore::new();
    let self_id = MemoryId::generate();
    store
        .append(
            Timestamp::from_millis(1_000),
            EventSource::Agent,
            vec![
                EventPayload::memory_created(self_id, MemoryName::new(MemoryName::SELF)),
                EventPayload::LinkTypeRegistered {
                    name: RelationName::CreatedBy,
                    inverse: RelationName::Created,
                    from_card: Cardinality::One,
                    to_card: Cardinality::Many,
                    symmetric: false,
                    reflexive: false,
                    description: String::new(),
                },
                EventPayload::LinkTypeRegistered {
                    name: RelationName::SameAs,
                    inverse: RelationName::SameAs,
                    from_card: Cardinality::Many,
                    to_card: Cardinality::Many,
                    symmetric: true,
                    reflexive: false,
                    description: String::new(),
                },
            ],
        )
        .unwrap();
    let mut graph = Graph::open_in_memory().unwrap();
    graph.materialize_from(&store).unwrap();
    (graph, self_id)
}

/// A block with a custom `max_entry_chars` limit, for the oversized-content tests.
pub(super) fn block_with_limit(
    graph: Graph,
    clock: ManualClock,
    max_entry_chars: usize,
) -> MemoryBlock {
    let engine = Engine::new(Box::new(MemoryStore::new()), graph, Box::new(clock));
    MemoryBlock::new(
        engine,
        Teller::Agent,
        Authority::Platform,
        Some(ConversationId::generate()),
        None,
        Vec::new(),
        max_entry_chars,
    )
    .unwrap()
}

/// A graph seeded with two merged person memories (`person/quinn` and `person/quinn@chat`, bound
/// by a committed `same_as`) and the `same_as` relation — enough to exercise the foreign-confidence
/// supersede guard's class resolution, where a confidence told by one identity is the other's own.
/// Returns the graph and the two ids.
pub(super) fn graph_with_merged_pair() -> (Graph, MemoryId, MemoryId) {
    let mut store = MemoryStore::new();
    let quinn = MemoryId::generate();
    let quinn_chat = MemoryId::generate();
    store
        .append(
            Timestamp::from_millis(1_000),
            EventSource::Agent,
            vec![
                EventPayload::LinkTypeRegistered {
                    name: RelationName::SameAs,
                    inverse: RelationName::SameAs,
                    from_card: Cardinality::Many,
                    to_card: Cardinality::Many,
                    symmetric: true,
                    reflexive: false,
                    description: String::new(),
                },
                EventPayload::memory_created(quinn, Namespace::Person.with_name("quinn")),
                EventPayload::memory_created(quinn_chat, Namespace::Person.with_name("quinn@chat")),
                EventPayload::link_created(
                    quinn,
                    quinn_chat,
                    RelationName::SameAs,
                    LinkPosture {
                        source: LinkSource::Operator,
                        told_by: None,
                        told_in: None,
                        visibility: Visibility::Public,
                    },
                ),
            ],
        )
        .unwrap();
    let mut graph = Graph::open_in_memory().unwrap();
    graph.materialize_from(&store).unwrap();
    (graph, quinn, quinn_chat)
}

/// An append recording a confidence with the given posture, told by a specific participant — the
/// shape the foreign-confidence supersede guard reasons over.
pub(super) fn told(teller: Teller, visibility: VisibilityChoice) -> AppendOptions {
    AppendOptions {
        visibility: Some(visibility),
        told_by: Some(teller),
        ..AppendOptions::default()
    }
}
