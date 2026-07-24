//! Join, presence, and identity resolution: noting participants arriving mid-session, resyncing a
//! room's full roster, ensuring a conversation exists, and resolving the agent's own `self` memory.

use crate::{
    graph::GraphError,
    ids::{ConversationId, ConversationLocator, MemoryId, PersonId},
    instance::{
        InstanceError,
        platform::{Platform, RosterResync},
    },
    memory::identity::resolve_or_mint_conversation,
    model::ModelClient,
};

impl Platform<'_> {
    /// Note a participant arriving mid-session — the explicit join path, for clients that deliver
    /// presence changes as their own signal (the per-message presence sync in the turn path covers
    /// those that only deliver messages). If the room has a live session, this records a
    /// `ParticipantJoined` and injects the joiner's brief — built against the now-present set, so the
    /// subject-guard suppresses asides about them — as a `system` turn at the join point, rather than
    /// rebuilding the frozen prompt (spec §Mid-conversation joins). A no-op if the room has never been
    /// seen or has no live session; the next message then opens a session with the joiner present.
    /// `model` feeds the joiner's describe catch-up before the brief composes; with none configured
    /// the brief composes off the current prose — a slightly stale join-brief beats refusing the join.
    pub async fn note_join(
        &self,
        model: Option<&dyn ModelClient>,
        locator: &ConversationLocator,
        participant: &PersonId,
    ) -> Result<(), InstanceError> {
        let Some(conversation) = self
            .server
            .engine
            .graph
            .lock()
            .conversation_for_locator(locator)?
        else {
            return Ok(());
        };
        let Some(session) = self.server.sessions.get(conversation).map(|open| open.id) else {
            return Ok(());
        };

        let joiner = self.resolve_or_mint_person(participant)?;
        self.server
            .join_participant(model, conversation, session, joiner)
            .await
    }

    /// Resolve `locator`'s conversation, minting it if this is the room's first contact — the same
    /// resolution `route_message` performs, exposed so the streaming message endpoint can subscribe
    /// to the room's progress frames before the turn begins. Idempotent: an existing conversation is
    /// returned untouched.
    pub fn ensure_conversation(
        &self,
        locator: &ConversationLocator,
    ) -> Result<ConversationId, InstanceError> {
        // One graph guard spans the mint and the materialization: released between them, a
        // concurrent first contact for the same locator would resolve against a graph that does
        // not yet hold the mint and mint the room a second time. Graph before store, per the
        // lock-ordering rule; the store is locked briefly twice within the span.
        let mut graph = self.server.engine.graph.lock();
        let id = resolve_or_mint_conversation(
            self.server.engine.store.lock().as_mut(),
            self.server.engine.clock.as_ref(),
            &graph,
            locator,
        )?;
        graph.materialize_from(self.server.engine.store.lock().as_ref())?;
        Ok(id)
    }

    /// Resync a room's full roster — the counterpart to `note_join` for a connector that observes
    /// presence directly (a voice channel's member list, a presence event) rather than only through
    /// messages. Given every platform id currently present, this diffs against the live session's
    /// members: each arrival routes through the same join machinery as `note_join` — a
    /// `ParticipantJoined` and an injected join-brief built against the now-present set — while a
    /// departure is acknowledged but records no event, because the message-carried present set is
    /// what drives per-turn visibility and membership drift is carried by each session's own present
    /// set (spec §Conversations and briefs → n is per-session). A no-op returning an empty resync if
    /// the room has never been seen or has no live session; the next message then opens a session
    /// with the current roster present. `model`, when configured, feeds each arrival's describe
    /// catch-up before its brief composes, as `note_join` does.
    pub async fn note_presence(
        &self,
        model: Option<&dyn ModelClient>,
        locator: &ConversationLocator,
        roster: &[PersonId],
    ) -> Result<RosterResync, InstanceError> {
        let Some(conversation) = self
            .server
            .engine
            .graph
            .lock()
            .conversation_for_locator(locator)?
        else {
            return Ok(RosterResync::default());
        };
        let Some(session) = self.server.sessions.get(conversation).map(|open| open.id) else {
            return Ok(RosterResync::default());
        };

        // Resolve the roster to memory ids, deduplicating first to spare the redundant
        // resolve-and-materialize cycle for a repeated id; each first-contact mint is made atomic by
        // `resolve_or_mint_person`.
        let mut uids: Vec<&PersonId> = Vec::new();
        for person in roster {
            if !uids.contains(&person) {
                uids.push(person);
            }
        }
        let mut present_ids = Vec::with_capacity(uids.len());
        for person in &uids {
            present_ids.push(self.resolve_or_mint_person(person)?);
        }

        // Diff against the session's members, read once. An id present but not a member is an arrival
        // to brief in; a member absent from the roster is a departure to acknowledge. Two distinct
        // platform ids can resolve to one memory (a merged cross-platform identity), so a joined-id
        // guard keeps a single arrival from being briefed twice within the pass.
        let members = self
            .server
            .engine
            .graph
            .lock()
            .session_participants(session)?;
        let mut joined = Vec::new();
        let mut joined_ids: Vec<MemoryId> = Vec::new();
        for (person, &id) in uids.iter().zip(present_ids.iter()) {
            if !members.contains(&id) && !joined_ids.contains(&id) {
                self.server
                    .join_participant(model, conversation, session, id)
                    .await?;
                joined.push((*person).clone());
                joined_ids.push(id);
            }
        }
        let departed = members
            .iter()
            .filter(|member| !present_ids.contains(member))
            .count();

        Ok(RosterResync { joined, departed })
    }

    /// The id of the agent's reserved `self` memory, resolved from the graph by its handle. A connector
    /// reads it to splice a `[mem:<id>]` reference when the agent itself is @mentioned — the same
    /// canonical memory token a mentioned participant's projection returns, so the agent's own mention
    /// reads as a reference rather than an opaque platform mention. `self` is minted at genesis, so its
    /// absence is an internal invariant failure, surfaced as a corrupt-projection graph error rather than
    /// a distinct variant.
    pub fn self_memory(&self) -> Result<MemoryId, InstanceError> {
        let self_memory = self.server.engine.graph.lock().self_memory()?;
        self_memory.map(|view| view.id).ok_or_else(|| {
            InstanceError::Graph(GraphError::Malformed(
                "the reserved `self` memory is absent, which cannot occur after genesis".to_owned(),
            ))
        })
    }
}
