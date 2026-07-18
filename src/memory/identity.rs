//! Resolve platform identities and conversation locators to their durable ids, minting on first
//! contact (spec §Identity, §Conversations).
//!
//! This is the single writer's boundary logic: a platform participant or a room locator the agent
//! has never seen is created here, lazily, the first time it appears — a participant as a
//! [`Namespace::Person`] stub bound to its `(platform, platform_user_id)` key, a room as a
//! `ConversationStarted`. The
//! resolvers append to the log and return the id; the caller materializes the graph so a freshly
//! minted id is visible to subsequent reads, mirroring the genesis-then-materialize discipline.

use crate::{
    clock::Clock,
    event::{EventPayload, EventSource},
    graph::{Graph, GraphError},
    ids::{ConversationId, ConversationLocator, MemoryId, MemoryName, Namespace},
    store::{Store, StoreError},
};

/// The provisional name of the [`Namespace::Context`] memory minted for a freshly opened room,
/// derived from its locator. The agent or operator renames it to a friendly handle
/// (`context/acme-leads`) later.
fn context_name(locator: &ConversationLocator) -> MemoryName {
    Namespace::Context
        .with_name(format!("{}:{}", locator.platform, locator.scope_path))
        .into()
}

/// A failure resolving or minting an identity, delegating to the store or graph beneath it. `context`
/// identifies the platform participant or room locator being resolved (e.g. `discord:12345` or
/// `discord:guild/42/chan/leads`), packed at the resolve-or-mint boundary so an operator seeing the
/// error knows which identity failed.
#[derive(Debug)]
pub enum IdentityError {
    Store { context: String, source: StoreError },
    Graph { context: String, source: GraphError },
}

impl IdentityError {
    fn with_context(self, context: String) -> Self {
        match self {
            IdentityError::Store { source, .. } => IdentityError::Store { context, source },
            IdentityError::Graph { source, .. } => IdentityError::Graph { context, source },
        }
    }
}

impl std::fmt::Display for IdentityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IdentityError::Store { context, source } => {
                if context.is_empty() {
                    write!(f, "identity (store): {source}")
                } else {
                    write!(f, "identity: {context} (store): {source}")
                }
            }
            IdentityError::Graph { context, source } => {
                if context.is_empty() {
                    write!(f, "identity (graph): {source}")
                } else {
                    write!(f, "identity: {context} (graph): {source}")
                }
            }
        }
    }
}

impl std::error::Error for IdentityError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            IdentityError::Store { source, .. } => Some(source),
            IdentityError::Graph { source, .. } => Some(source),
        }
    }
}

impl From<StoreError> for IdentityError {
    fn from(source: StoreError) -> IdentityError {
        IdentityError::Store {
            context: String::new(),
            source,
        }
    }
}

impl From<GraphError> for IdentityError {
    fn from(source: GraphError) -> IdentityError {
        IdentityError::Graph {
            context: String::new(),
            source,
        }
    }
}

/// Resolve a platform participant to their [`Namespace::Person`] memory, minting one on first
/// contact. Returns the memory's id (the caller materializes the graph to see a freshly minted
/// one). A mint appends a `MemoryCreated` and a `ParticipantIdentified` binding the
/// `(platform, platform_user_id)` key to it, unless the qualified name already exists as a memory
/// (an agent-authored hearsay stub), in which case only a `ParticipantIdentified` is emitted to
/// bind the platform identity to the existing stub.
pub fn resolve_or_mint_participant(
    store: &mut dyn Store,
    clock: &dyn Clock,
    graph: &Graph,
    platform: &str,
    platform_user_id: &str,
) -> Result<MemoryId, IdentityError> {
    let context = format!("{platform}:{platform_user_id}");
    (|| -> Result<MemoryId, IdentityError> {
        if let Some(id) = graph.participant_for(platform, platform_user_id)? {
            return Ok(id);
        }
        let mint = graph.participant_mint(platform, platform_user_id)?;
        // If the qualified name already exists as a memory (an agent-authored stub), bind the
        // platform identity to it. Otherwise, mint a fresh memory.
        let (id, mut events) = match graph.memory_by_name(&mint.name)? {
            Some(existing) => {
                tracing::info!(
                    %platform, %platform_user_id, memory = %existing.id.0,
                    "bound platform identity to an existing unbound stub",
                );
                (existing.id, Vec::new())
            }
            None => {
                let id = MemoryId::generate();
                (id, vec![EventPayload::memory_created(id, mint.name.clone())])
            }
        };
        events.push(EventPayload::participant_identified(id, platform, platform_user_id));
        store.append(clock.now(), EventSource::Orchestration, events)?;
        tracing::info!(%platform, %platform_user_id, memory = %id.0, name = %mint.name.as_str(), "minted participant");
        Ok(id)
    })()
    .map_err(|e| e.with_context(context))
}

/// Resolve a scope to its [`Namespace::Context`] memory, minting one on first contact. The context
/// memory is keyed by its name (`context/<platform>:<scope_path>`), independent of any conversation:
/// a room's context and a standalone context — a guild that hosts channels but has no messages of its
/// own — resolve the same way, so a platform connector can establish context for a scope that never becomes a
/// conversation. A room's [`resolve_or_mint_conversation`] reuses this same memory by name, so a scope
/// has exactly one context memory whichever path first mints it (spec §Contexts are first-class
/// memories). The caller materializes the graph to see a freshly minted one.
pub fn resolve_or_mint_context(
    store: &mut dyn Store,
    clock: &dyn Clock,
    graph: &Graph,
    locator: &ConversationLocator,
) -> Result<MemoryId, IdentityError> {
    let name = context_name(locator);
    if let Some(existing) = graph.memory_by_name(&name)? {
        return Ok(existing.id);
    }
    let id = MemoryId::generate();
    store.append(
        clock.now(),
        EventSource::Orchestration,
        vec![EventPayload::memory_created(id, name)],
    )?;
    Ok(id)
}

/// Resolve a scope to its [`Namespace::Context`] memory without minting one — `None` when the scope
/// has no context memory yet. The retract counterpart to [`resolve_or_mint_context`]: a platform connector
/// removing a structural link resolves the nodes it already established rather than minting them
/// afresh, so a retract naming an unknown scope is a no-op rather than a pointless mint.
pub fn resolve_context(
    graph: &Graph,
    locator: &ConversationLocator,
) -> Result<Option<MemoryId>, GraphError> {
    Ok(graph.memory_by_name(context_name(locator))?.map(|m| m.id))
}

/// Resolve a room locator to its conversation, opening one on first contact. Returns the
/// conversation's id (the caller materializes the graph to see a freshly opened room). Opening a room
/// resolves its [`Namespace::Context`] memory via [`resolve_or_mint_context`] — reusing one already
/// established for the scope (e.g. by an earlier context write) rather than minting a duplicate — and
/// records it on the `ConversationStarted`, so the locator resolves to a first-class memory the agent
/// can tag and reason about (spec §Contexts are first-class memories).
pub fn resolve_or_mint_conversation(
    store: &mut dyn Store,
    clock: &dyn Clock,
    graph: &Graph,
    locator: &ConversationLocator,
) -> Result<ConversationId, IdentityError> {
    let context = format!("{}:{}", locator.platform, locator.scope_path);
    (|| -> Result<ConversationId, IdentityError> {
        if let Some(id) = graph.conversation_for_locator(locator)? {
            return Ok(id);
        }
        let context_memory = resolve_or_mint_context(store, clock, graph, locator)?;
        let id = ConversationId::generate();
        store.append(
            clock.now(),
            EventSource::Orchestration,
            vec![EventPayload::conversation_started(
                id,
                locator.clone(),
                context_memory,
            )],
        )?;
        tracing::info!(
            platform = %locator.platform,
            scope_path = %locator.scope_path,
            conversation = %id.0,
            context = %context_memory.0,
            "opened conversation"
        );
        Ok(id)
    })()
    .map_err(|e| e.with_context(context))
}

#[cfg(test)]
mod tests {
    //! A platform participant and a room locator resolve to a stable id, minting exactly once on
    //! first contact and resolving to the same id thereafter (spec §Identity).
    use super::{resolve_or_mint_conversation, resolve_or_mint_participant};
    use crate::{
        TEST_PLATFORM, TEST_PLATFORM_ALT,
        clock::ManualClock,
        event::{EventPayload, EventSource},
        graph::Graph,
        ids::{ConversationLocator, MemoryId, MemoryName, Namespace, Seq},
        store::{MemoryStore, Store},
        time::Timestamp,
    };

    #[test]
    fn participant_resolves_and_mints_once() {
        let mut store = MemoryStore::new();
        let clock = ManualClock::new(Timestamp::from_millis(1_000));
        let mut graph = Graph::open_in_memory().unwrap();

        // First contact mints a qualified handle bound to the platform key.
        let id = resolve_or_mint_participant(&mut store, &clock, &graph, TEST_PLATFORM, "12345")
            .unwrap();
        graph.materialize_from(&store).unwrap();
        assert_eq!(store.head().unwrap(), Seq(2)); // MemoryCreated + ParticipantIdentified
        assert_eq!(
            graph.participant_for(TEST_PLATFORM, "12345").unwrap(),
            Some(id)
        );
        assert_eq!(
            graph.memory_by_id(id).unwrap().unwrap().name,
            Namespace::Person.with_name("12345@chat").into()
        );

        // Second contact resolves to the same memory and mints nothing.
        let again = resolve_or_mint_participant(&mut store, &clock, &graph, TEST_PLATFORM, "12345")
            .unwrap();
        assert_eq!(again, id);
        assert_eq!(store.head().unwrap(), Seq(2));

        // A different platform user gets its own qualified handle.
        let other = resolve_or_mint_participant(&mut store, &clock, &graph, TEST_PLATFORM, "67890")
            .unwrap();
        // The same user_id on another platform gets a different qualified name, so no collision.
        let elsewhere =
            resolve_or_mint_participant(&mut store, &clock, &graph, TEST_PLATFORM_ALT, "12345")
                .unwrap();
        graph.materialize_from(&store).unwrap();
        assert_ne!(other, id);
        assert_ne!(elsewhere, id);
        assert_ne!(elsewhere, other);
        assert_eq!(store.head().unwrap(), Seq(6)); // two more mints, two events each
        assert_eq!(
            graph.memory_by_id(other).unwrap().unwrap().name,
            Namespace::Person.with_name("67890@chat").into()
        );
        assert_eq!(
            graph.memory_by_id(elsewhere).unwrap().unwrap().name,
            Namespace::Person.with_name("12345@forum").into()
        );
    }

    #[test]
    fn arrival_matching_an_unbound_stub_binds_to_it() {
        let mut store = MemoryStore::new();
        let clock = ManualClock::new(Timestamp::from_millis(1_000));
        let mut graph = Graph::open_in_memory().unwrap();

        // An agent-authored stub: `person/nadia@chat` exists but is bound to no platform.
        let hearsay = MemoryId::generate();
        store
            .append(
                Timestamp::from_millis(1_000),
                EventSource::Agent,
                vec![EventPayload::memory_created(
                    hearsay,
                    Namespace::Person.with_name("nadia@chat"),
                )],
            )
            .unwrap();
        graph.materialize_from(&store).unwrap();

        // Nadia then arrives on chat: the qualified name matches the unbound stub, so the
        // platform identity is bound to it — no new memory is created, no merge proposed.
        let arrival =
            resolve_or_mint_participant(&mut store, &clock, &graph, TEST_PLATFORM, "nadia")
                .unwrap();
        graph.materialize_from(&store).unwrap();
        assert_eq!(arrival, hearsay, "the arrival binds to the existing stub");
        assert_eq!(
            graph.memory_by_id(arrival).unwrap().unwrap().name,
            Namespace::Person.with_name("nadia@chat").into()
        );

        // No merge proposals or same_as links — the stub and the arrival are the same memory.
        for event in store.read_from(Seq::ZERO).unwrap() {
            assert!(
                !matches!(event.payload, EventPayload::MergeProposed { .. }),
                "no merge proposal should be created"
            );
            assert!(
                !matches!(event.payload, EventPayload::LinkCreated { .. }),
                "no link should be created"
            );
        }
    }

    #[test]
    fn a_direct_arrival_matching_an_unbound_stub_binds_to_it() {
        let mut store = MemoryStore::new();
        let clock = ManualClock::new(Timestamp::from_millis(1_000));
        let mut graph = Graph::open_in_memory().unwrap();

        // An agent-authored stub: `person/nadia@direct` exists, bound to no platform.
        let hearsay = MemoryId::generate();
        store
            .append(
                Timestamp::from_millis(1_000),
                EventSource::Agent,
                vec![EventPayload::memory_created(
                    hearsay,
                    Namespace::Person.with_name("nadia@direct"),
                )],
            )
            .unwrap();
        graph.materialize_from(&store).unwrap();

        // Nadia arrives through the operator's own direct interface: the qualified name matches
        // the unbound stub, so the platform identity binds to it. No merge or link needed —
        // the stub and the arrival are the same memory.
        let arrival =
            resolve_or_mint_participant(&mut store, &clock, &graph, "direct", "nadia").unwrap();
        graph.materialize_from(&store).unwrap();
        assert_eq!(arrival, hearsay, "the arrival binds to the existing stub");

        // No merge proposals or same_as links.
        for event in store.read_from(Seq::ZERO).unwrap() {
            assert!(
                !matches!(event.payload, EventPayload::MergeProposed { .. }),
                "no merge proposal should be created"
            );
            assert!(
                !matches!(event.payload, EventPayload::LinkCreated { .. }),
                "no link should be created"
            );
        }
    }

    #[test]
    fn conversation_resolves_and_mints_once() {
        let mut store = MemoryStore::new();
        let clock = ManualClock::new(Timestamp::from_millis(1_000));
        let mut graph = Graph::open_in_memory().unwrap();
        let leads = ConversationLocator::new(TEST_PLATFORM, "guild/42/chan/leads");

        // First contact opens the room and eagerly mints its context memory.
        let id = resolve_or_mint_conversation(&mut store, &clock, &graph, &leads).unwrap();
        graph.materialize_from(&store).unwrap();
        assert_eq!(store.head().unwrap(), Seq(2)); // MemoryCreated(context) + ConversationStarted
        assert_eq!(graph.conversation_for_locator(&leads).unwrap(), Some(id));
        // The locator resolves to a real, non-person context memory (defaults Public, no subject-guard).
        let context = graph.context_for_conversation(id).unwrap().unwrap();
        let context_name =
            MemoryName::from(Namespace::Context.with_name("chat:guild/42/chan/leads"));
        assert_eq!(
            graph.memory_by_id(context).unwrap().unwrap().name.as_str(),
            context_name.as_str()
        );

        // The same locator resolves to the same room and opens nothing new.
        let again = resolve_or_mint_conversation(&mut store, &clock, &graph, &leads).unwrap();
        assert_eq!(again, id);
        assert_eq!(store.head().unwrap(), Seq(2));

        // A different room is a distinct conversation with its own context.
        let dms = ConversationLocator::new(TEST_PLATFORM, "dm/dave");
        let other = resolve_or_mint_conversation(&mut store, &clock, &graph, &dms).unwrap();
        assert_ne!(other, id);
        assert_eq!(store.head().unwrap(), Seq(4));
    }
}
