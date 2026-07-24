//! The platform-authority facet: a client delivering participant turns, and the compaction the turn
//! loop triggers. It can act only as the participants it represents and cannot reach Control's
//! operator surface — the structural absence of those methods is what makes "the operator has no
//! platform identity" enforceable (spec §Clients and the server boundary).

use serde::{Deserialize, Serialize};

use crate::{
    agent::TurnView,
    graph::GraphError,
    ids::{ConversationLocator, EntryId, MemoryId, PersonId},
    instance::{Instance, InstanceError},
    memory::memory_block::{MemoryBlock, MemoryError},
    store::StoreError,
    vocabulary::RelationName,
};

mod links;
mod presence;
mod projection;
mod routing;

/// One inbound participant message in a batch delivered to `route_messages`.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct MessageInput {
    /// The sender's platform identity.
    pub sender: PersonId,
    /// The message text.
    pub text: String,
}

/// One attribute projected onto a scoped memory via [`Platform::project`] — a participant's username,
/// display name, or nickname, or a guild's name. `text` is the value
/// to record now, or `None` to clear a value that is no longer set. `supersedes` is the entry id a
/// prior projection of this same attribute returned, so a changed value supersedes it and a cleared
/// one retracts it — the connector holds that id, so the server needs no per-attribute keying.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ParticipantAttribute {
    /// The attribute's current value, or `None` to clear the prior value.
    pub text: Option<String>,
    /// The entry a prior projection of this attribute returned, to supersede or retract.
    pub supersedes: Option<EntryId>,
}

/// The outcome of a [`Platform::project`] call: the id of the memory the attributes landed on (resolved
/// or minted), and the new entry id per attribute in request order — `Some` for a recorded value, `None`
/// for a cleared or absent one. A connector holds the memory id to splice a `[mem:<id>]` reference for
/// the subject (an @mention rewritten to a canonical memory token) without a round trip on an unchanged
/// identity, and the entry ids to supersede on the next change.
#[derive(Clone, Debug, Serialize)]
pub struct ProjectOutcome {
    /// The memory the projection landed on, resolved or minted from the target.
    pub memory_id: MemoryId,
    /// The new entry id per attribute, in request order.
    pub entries: Vec<Option<EntryId>>,
}

/// One endpoint of a connector-authored structural link ([`Platform::link`]) — a participant or a
/// context, each named under the connector's own platform. A connector can only ever link memories it
/// owns, so both nodes are scoped to its platform by construction.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum LinkNode {
    /// A participant's `person/*` stub, by platform identity.
    Participant(PersonId),
    /// A scope's `context/*` memory, by locator (a guild, a channel).
    Context(ConversationLocator),
}

/// A failure asserting or retracting a connector-authored link. The first two are client-contract
/// violations a connector should never send — a `400` for the connector to fix — distinct from an
/// underlying store or graph failure.
#[derive(Debug)]
pub enum LinkError {
    /// A connector may not assert `same_as`: cross-platform identity is operator-confirmed, never a
    /// connector's to assert (spec §Cross-platform identity is operator-asserted only).
    SameAsForbidden,
    /// The named relation is not registered in the ontology, so the edge would be mis-typed.
    UnknownRelation(RelationName),
    /// An underlying instance failure resolving the endpoints or appending the edge.
    Instance(InstanceError),
}

impl std::fmt::Display for LinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LinkError::SameAsForbidden => write!(
                f,
                "platform link: a platform connector may not assert same_as; cross-platform identity \
                 is operator-confirmed"
            ),
            LinkError::UnknownRelation(relation) => write!(
                f,
                "platform link: unknown relation {:?}; a platform connector may link only registered \
                 relations",
                relation.as_str()
            ),
            LinkError::Instance(error) => write!(f, "platform link: {error}"),
        }
    }
}

impl std::error::Error for LinkError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            LinkError::SameAsForbidden | LinkError::UnknownRelation(_) => None,
            LinkError::Instance(error) => Some(error),
        }
    }
}

impl From<InstanceError> for LinkError {
    fn from(error: InstanceError) -> Self {
        LinkError::Instance(error)
    }
}

impl From<StoreError> for LinkError {
    fn from(error: StoreError) -> Self {
        LinkError::Instance(InstanceError::Store(error))
    }
}

impl From<GraphError> for LinkError {
    fn from(error: GraphError) -> Self {
        LinkError::Instance(InstanceError::Graph(error))
    }
}

/// Platform-authority operations: a client delivering participant turns. It can act only as the
/// participants it represents, and cannot reach Control's operator surface.
pub struct Platform<'a> {
    pub(super) server: &'a Instance,
}

/// The outcome of a roster resync ([`Platform::note_presence`]): the arrivals it briefed into the
/// live session, and how many prior members the new roster no longer lists.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RosterResync {
    /// The platform user ids that were newly present. Each received a `ParticipantJoined` and an
    /// injected join-brief, exactly as an explicit [`Platform::note_join`] would.
    pub joined: Vec<PersonId>,
    /// The count of the session's prior members absent from the new roster. Departures are
    /// deliberately eventless (spec §Conversations and briefs → n is per-session): the per-message
    /// present set drives visibility, so a departed participant stops affecting retrieval on the
    /// next message with no event of its own, and membership drift is carried by each session's own
    /// present set. The count is reported for the connector's confirmation and for observability,
    /// never recorded.
    pub departed: usize,
}

/// Supersede `old` by `new` on `memory`, treating an `old` the agent has already dropped as a no-op.
/// A projection's supersede target can vanish between projections — the agent may have retracted or
/// superseded it — so a missing target leaves the fresh append standing rather than failing the write.
pub(super) fn supersede_if_live(
    block: &mut MemoryBlock,
    memory: MemoryId,
    old: EntryId,
    new: EntryId,
) -> Result<(), InstanceError> {
    match block.supersede(memory, old, new) {
        Ok(()) | Err(MemoryError::UnknownEntry(_)) => Ok(()),
        Err(e) => Err(InstanceError::Memory(e)),
    }
}

/// Retract `old` on `memory`, treating an `old` the agent has already dropped as a no-op — the cleared
/// attribute's target may no longer be live, exactly as in [`supersede_if_live`].
pub(super) fn retract_if_live(
    block: &mut MemoryBlock,
    memory: MemoryId,
    old: EntryId,
) -> Result<(), InstanceError> {
    match block.retract(memory, old, "no longer set on the platform.", None) {
        Ok(_) | Err(MemoryError::UnknownEntry(_)) => Ok(()),
        Err(e) => Err(InstanceError::Memory(e)),
    }
}

/// A deterministic `chars / 4` estimate of the prompt's token count over the buffer plus the inbound
/// message — the compaction-trigger fallback when the backend reports no usage. Coarse and an
/// under-count (it omits the frozen prefix); only the real client's `prompt_tokens` is authoritative.
pub(super) fn estimate_tokens(buffer: &[TurnView], messages: &[MessageInput]) -> i64 {
    let chars: usize = buffer
        .iter()
        .map(|turn| turn.text.chars().count())
        .sum::<usize>()
        + messages
            .iter()
            .map(|m| m.text.chars().count())
            .sum::<usize>();
    (chars / 4) as i64
}

#[cfg(test)]
mod tests;
