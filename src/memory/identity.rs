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
    event::{EventPayload, MergeProposalSource},
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
/// one). A mint appends a
/// `MemoryCreated` and a `ParticipantIdentified` binding the `(platform, platform_user_id)` key to it.
///
/// The name is the clean `person/<platform_user_id>`, so a person is one coherent memory the agent
/// reads and writes under a single handle — not split between a system stub and a canonical the agent
/// mints alongside it. The platform-qualified `person/<platform_user_id>@<platform>` form is used only
/// when that clean name is already taken (spec §Identity → cross-platform-explicit): if it belongs to a
/// *different* platform-bound identity (the same handle on two platforms), the qualified stub keeps two
/// distinct people distinct rather than silently merging; if it belongs to a platform-*unbound* memory
/// (an agent-authored hearsay stub the agent wrote from conversation, never bound to a platform), the
/// qualified stub is still minted, but a `MergeProposed` (`Orchestration`-sourced) is emitted alongside
/// it so the adjudicator or operator can reunite the two — a handle match never itself asserts identity
/// (the impersonation surface). The `(platform, key)` binding lives in `ParticipantIdentified`
/// regardless of the name, so the name stays free to be the clean one and to be renamed later
/// (humanizing a raw id) without breaking resolution.
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
        let id = MemoryId::generate();
        let mint = graph.participant_mint(platform, platform_user_id)?;
        let name = mint.name;
        let mut events = vec![
            EventPayload::memory_created(id, name.clone()),
            EventPayload::participant_identified(id, platform, platform_user_id),
        ];
        if let Some(existing) = mint.propose_same_as_with {
            // The clean handle is an unbound hearsay stub: propose reuniting the fresh platform-bound
            // stub with it, for the adjudicator or operator to weigh. Never an auto-merge — the handle
            // match alone is not identity.
            events.push(EventPayload::merge_proposed(
                id,
                existing,
                MergeProposalSource::Orchestration,
                None,
            ));
            tracing::info!(
                %platform, %platform_user_id, memory = %id.0, existing = %existing.0,
                "proposed merging a platform arrival with a matching unbound stub",
            );
        }
        store.append(clock.now(), events)?;
        tracing::info!(%platform, %platform_user_id, memory = %id.0, name = %name.as_str(), "minted participant");
        Ok(id)
    })()
    .map_err(|e| e.with_context(context))
}

/// Resolve a room locator to its conversation, opening one on first contact. Returns the
/// conversation's id (the caller materializes the graph to see a freshly opened room). Opening a
/// room eagerly mints its [`Namespace::Context`] memory under a provisional locator-derived name
/// and records it on the `ConversationStarted`, so the locator resolves to a first-class memory
/// the agent can
/// tag and reason about (spec §Contexts are first-class memories).
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
        let id = ConversationId::generate();
        let context_memory = MemoryId::generate();
        store.append(
            clock.now(),
            vec![
                EventPayload::memory_created(context_memory, context_name(locator)),
                EventPayload::conversation_started(id, locator.clone(), context_memory),
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
    })()
    .map_err(|e| e.with_context(context))
}

#[cfg(test)]
mod tests {
    //! A platform participant and a room locator resolve to a stable id, minting exactly once on
    //! first contact and resolving to the same id thereafter (spec §Identity).
    use super::{resolve_or_mint_conversation, resolve_or_mint_participant};
    use crate::{
        clock::ManualClock,
        event::{EventPayload, MergeProposalSource},
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

        // First contact mints a clean handle (no platform suffix) bound to the platform key.
        let id =
            resolve_or_mint_participant(&mut store, &clock, &graph, "discord", "12345").unwrap();
        graph.materialize_from(&store).unwrap();
        assert_eq!(store.head().unwrap(), Seq(2)); // MemoryCreated + ParticipantIdentified
        assert_eq!(graph.participant_for("discord", "12345").unwrap(), Some(id));
        assert_eq!(
            graph.memory_by_id(id).unwrap().unwrap().name,
            Namespace::Person.with_name("12345").into()
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
            graph.memory_by_id(other).unwrap().unwrap().name,
            Namespace::Person.with_name("67890").into()
        );
        assert_eq!(
            graph.memory_by_id(elsewhere).unwrap().unwrap().name,
            Namespace::Person.with_name("12345@slack").into()
        );
    }

    #[test]
    fn arrival_matching_an_unbound_stub_mints_qualified_and_proposes_a_merge() {
        let mut store = MemoryStore::new();
        let clock = ManualClock::new(Timestamp::from_millis(1_000));
        let mut graph = Graph::open_in_memory().unwrap();

        // An agent-authored hearsay stub: `person/nadia` exists but is bound to no platform.
        let hearsay = MemoryId::generate();
        store
            .append(
                Timestamp::from_millis(1_000),
                vec![EventPayload::memory_created(
                    hearsay,
                    Namespace::Person.with_name("nadia"),
                )],
            )
            .unwrap();
        graph.materialize_from(&store).unwrap();

        // Nadia then arrives on a platform: the qualified stub is minted (not merged onto the hearsay
        // one), and an orchestration-sourced merge is proposed to reunite them for adjudication.
        let arrival =
            resolve_or_mint_participant(&mut store, &clock, &graph, "discord", "nadia").unwrap();
        graph.materialize_from(&store).unwrap();
        assert_ne!(arrival, hearsay);
        assert_eq!(
            graph.memory_by_id(arrival).unwrap().unwrap().name,
            Namespace::Person.with_name("nadia@discord").into()
        );

        let proposals: Vec<_> = store
            .read_from(Seq::ZERO)
            .unwrap()
            .into_iter()
            .filter_map(|event| match event.payload {
                EventPayload::MergeProposed {
                    from, to, source, ..
                } => Some((from, to, source)),
                _ => None,
            })
            .collect();
        assert_eq!(
            proposals,
            vec![(arrival, hearsay, MergeProposalSource::Orchestration)]
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
        let context_name =
            MemoryName::from(Namespace::Context.with_name("discord:guild/42/chan/leads"));
        assert_eq!(
            graph.memory_by_id(context).unwrap().unwrap().name.as_str(),
            context_name.as_str()
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
