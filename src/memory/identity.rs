//! Resolve platform identities and conversation locators to their durable ids, minting on first
//! contact (spec §Identity, §Conversations).
//!
//! This is the single writer's boundary logic: a platform participant or a room locator the agent
//! has never seen is created here, lazily, the first time it appears — a participant as a `person/*`
//! stub bound to its `(platform, platform_user_id)` key, a room as a `ConversationStarted`. The
//! resolvers append to the log and return the id; the caller materializes the graph so a freshly
//! minted id is visible to subsequent reads, mirroring the genesis-then-materialize discipline.

use crate::{
    clock::Clock,
    event::EventPayload,
    graph::{Graph, GraphError},
    ids::{ConversationId, ConversationLocator, MemoryId, MemoryName},
    store::{Store, StoreError},
};

/// The provisional name of the `context/*` memory minted for a freshly opened room, derived from its
/// locator. The agent or operator renames it to a friendly handle (`context/acme-leads`) later.
fn context_name(locator: &ConversationLocator) -> MemoryName {
    MemoryName::new(format!(
        "context/{}:{}",
        locator.platform, locator.scope_path
    ))
}

/// A failure resolving or minting an identity, delegating to the store or graph beneath it.
#[derive(Debug)]
pub enum IdentityError {
    Store(StoreError),
    Graph(GraphError),
}

impl std::fmt::Display for IdentityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IdentityError::Store(error) => write!(f, "identity (store): {error}"),
            IdentityError::Graph(error) => write!(f, "identity (graph): {error}"),
        }
    }
}

impl std::error::Error for IdentityError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            IdentityError::Store(error) => Some(error),
            IdentityError::Graph(error) => Some(error),
        }
    }
}

impl From<StoreError> for IdentityError {
    fn from(error: StoreError) -> IdentityError {
        IdentityError::Store(error)
    }
}

impl From<GraphError> for IdentityError {
    fn from(error: GraphError) -> IdentityError {
        IdentityError::Graph(error)
    }
}

/// Resolve a platform participant to their `person/*` memory, minting one on first contact. Returns
/// the memory's id (the caller materializes the graph to see a freshly minted one). A mint appends a
/// `MemoryCreated` and a `ParticipantIdentified` binding the `(platform, platform_user_id)` key to it.
///
/// The name is the clean `person/<platform_user_id>`, so a person is one coherent memory the agent
/// reads and writes under a single handle — not split between a system stub and a canonical the agent
/// mints alongside it. The platform-qualified `person/<platform_user_id>@<platform>` form is used only
/// to disambiguate a genuine collision: when that clean name already belongs to a *different* identity
/// (the same handle on two platforms), so two distinct people stay distinct rather than silently
/// merging — the cross-platform-explicit property. The `(platform, key)` binding lives in
/// `ParticipantIdentified` regardless of the name, so the name stays free to be the clean one and to be
/// renamed later (humanizing a raw id) without breaking resolution.
pub fn resolve_or_mint_participant(
    store: &mut dyn Store,
    clock: &dyn Clock,
    graph: &Graph,
    platform: &str,
    platform_user_id: &str,
) -> Result<MemoryId, IdentityError> {
    if let Some(id) = graph.participant_for(platform, platform_user_id)? {
        return Ok(id);
    }
    let id = MemoryId::generate();
    let clean = format!("person/{platform_user_id}");
    let name = if graph.memory_by_name(&clean)?.is_some() {
        MemoryName::new(format!("person/{platform_user_id}@{platform}"))
    } else {
        MemoryName::new(clean)
    };
    store.append(
        clock.now(),
        vec![
            EventPayload::MemoryCreated {
                id,
                name: name.clone(),
            },
            EventPayload::ParticipantIdentified {
                memory: id,
                platform: platform.into(),
                platform_user_id: platform_user_id.into(),
            },
        ],
    )?;
    tracing::info!(%platform, %platform_user_id, memory = %id.0, name = %name.as_str(), "minted participant");
    Ok(id)
}

/// Resolve a room locator to its conversation, opening one on first contact. Returns the
/// conversation's id (the caller materializes the graph to see a freshly opened room). Opening a
/// room eagerly mints its `context/*` memory under a provisional locator-derived name and records
/// it on the `ConversationStarted`, so the locator resolves to a first-class memory the agent can
/// tag and reason about (spec §Contexts are first-class memories).
pub fn resolve_or_mint_conversation(
    store: &mut dyn Store,
    clock: &dyn Clock,
    graph: &Graph,
    locator: &ConversationLocator,
) -> Result<ConversationId, IdentityError> {
    if let Some(id) = graph.conversation_for_locator(locator)? {
        return Ok(id);
    }
    let id = ConversationId::generate();
    let context_memory = MemoryId::generate();
    store.append(
        clock.now(),
        vec![
            EventPayload::MemoryCreated {
                id: context_memory,
                name: context_name(locator),
            },
            EventPayload::ConversationStarted {
                id,
                locator: locator.clone(),
                context_memory,
            },
        ],
    )?;
    tracing::info!(
        platform = %locator.platform,
        scope_path = %locator.scope_path,
        conversation = %id.0,
        context = %context_memory.0,
        "opened conversation"
    );
    Ok(id)
}

#[cfg(test)]
mod tests {
    //! A platform participant and a room locator resolve to a stable id, minting exactly once on
    //! first contact and resolving to the same id thereafter (spec §Identity).
    use super::{resolve_or_mint_conversation, resolve_or_mint_participant};
    use crate::{
        clock::ManualClock,
        graph::Graph,
        ids::{ConversationLocator, Seq},
        store::{MemoryStore, Store},
        time::Timestamp,
    };

    #[test]
    fn participant_resolves_and_mints_once() {
        let mut store = MemoryStore::new();
        let clock = ManualClock::new(Timestamp::from_millis(1_000));
        let mut graph = Graph::open_in_memory().unwrap();

        // First contact mints a clean handle (no platform suffix) bound to the platform key.
        let id =
            resolve_or_mint_participant(&mut store, &clock, &graph, "discord", "12345").unwrap();
        graph.materialize_from(&store).unwrap();
        assert_eq!(store.head().unwrap(), Seq(2)); // MemoryCreated + ParticipantIdentified
        assert_eq!(graph.participant_for("discord", "12345").unwrap(), Some(id));
        assert_eq!(
            graph.memory_by_id(id).unwrap().unwrap().name.as_str(),
            "person/12345"
        );

        // Second contact resolves to the same memory and mints nothing.
        let again =
            resolve_or_mint_participant(&mut store, &clock, &graph, "discord", "12345").unwrap();
        assert_eq!(again, id);
        assert_eq!(store.head().unwrap(), Seq(2));

        // A different platform user gets its own clean handle.
        let other =
            resolve_or_mint_participant(&mut store, &clock, &graph, "discord", "67890").unwrap();
        // The same user_id on another platform collides with the clean name, so it disambiguates by
        // platform rather than silently merging two distinct people onto one handle.
        let elsewhere =
            resolve_or_mint_participant(&mut store, &clock, &graph, "slack", "12345").unwrap();
        graph.materialize_from(&store).unwrap();
        assert_ne!(other, id);
        assert_ne!(elsewhere, id);
        assert_ne!(elsewhere, other);
        assert_eq!(store.head().unwrap(), Seq(6)); // two more mints, two events each
        assert_eq!(
            graph.memory_by_id(other).unwrap().unwrap().name.as_str(),
            "person/67890"
        );
        assert_eq!(
            graph
                .memory_by_id(elsewhere)
                .unwrap()
                .unwrap()
                .name
                .as_str(),
            "person/12345@slack"
        );
    }

    #[test]
    fn conversation_resolves_and_mints_once() {
        let mut store = MemoryStore::new();
        let clock = ManualClock::new(Timestamp::from_millis(1_000));
        let mut graph = Graph::open_in_memory().unwrap();
        let leads = ConversationLocator::new("discord", "guild/42/chan/leads");

        // First contact opens the room and eagerly mints its context memory.
        let id = resolve_or_mint_conversation(&mut store, &clock, &graph, &leads).unwrap();
        graph.materialize_from(&store).unwrap();
        assert_eq!(store.head().unwrap(), Seq(2)); // MemoryCreated(context) + ConversationStarted
        assert_eq!(graph.conversation_for_locator(&leads).unwrap(), Some(id));
        // The locator resolves to a real, non-person context memory (defaults Public, no subject-guard).
        let context = graph.context_for_conversation(id).unwrap().unwrap();
        assert_eq!(
            graph.memory_by_id(context).unwrap().unwrap().name.as_str(),
            "context/discord:guild/42/chan/leads"
        );

        // The same locator resolves to the same room and opens nothing new.
        let again = resolve_or_mint_conversation(&mut store, &clock, &graph, &leads).unwrap();
        assert_eq!(again, id);
        assert_eq!(store.head().unwrap(), Seq(2));

        // A different room is a distinct conversation with its own context.
        let dms = ConversationLocator::new("discord", "dm/dave");
        let other = resolve_or_mint_conversation(&mut store, &clock, &graph, &dms).unwrap();
        assert_ne!(other, id);
        assert_eq!(store.head().unwrap(), Seq(4));
    }
}
