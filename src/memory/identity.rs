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

/// Resolve a platform participant to their `person/*` stub, minting one on first contact. Returns
/// the stub's id (the caller materializes the graph to see a freshly minted stub). A mint appends a
/// `MemoryCreated` under a provisional `person/<platform_user_id>@<platform>` name — which the
/// operator renames or merges later — and a `ParticipantIdentified` binding the platform key to it.
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
    let name = MemoryName::new(format!("person/{platform_user_id}@{platform}"));
    store.append(
        clock.now(),
        vec![
            EventPayload::MemoryCreated { id, name },
            EventPayload::ParticipantIdentified {
                memory: id,
                platform: platform.into(),
                platform_user_id: platform_user_id.into(),
            },
        ],
    )?;
    tracing::info!(%platform, %platform_user_id, memory = %id.0, "minted participant stub");
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
