//! The platform-authority facet: a client delivering participant turns, and the compaction the turn
//! loop triggers. It can act only as the participants it represents and cannot reach Control's
//! operator surface — the structural absence of those methods is what makes "the operator has no
//! platform identity" enforceable (spec §Clients and the server boundary).

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::{
    agent::{InboundMessage, TurnError, TurnOutcome, TurnView},
    event::{
        EventPayload, EventSource, LinkPosture, LinkSource, PromptTemplateName, SessionEndCause,
        Teller, Visibility,
    },
    graph::GraphError,
    ids::{ConversationId, ConversationLocator, EntryId, MemoryId, PersonId, TurnId},
    instance::{ContextEntry, Instance, InstanceError, RoutedTurn},
    memory::{
        identity::{
            resolve_context, resolve_or_mint_context, resolve_or_mint_conversation,
            resolve_or_mint_participant,
        },
        memory_block::{AppendOptions, Authority, MemoryBlock, MemoryError, VisibilityChoice},
    },
    model::ModelClient,
    settings::Settings,
    store::StoreError,
    vocabulary::RelationName,
};
use zuihitsu_connector_types::PlatformResponse;

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
    /// A connector may not assert `same_as`: cross-platform identity is operator-adjudicated, never a
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
                "platform link: a connector may not assert same_as; cross-platform identity is \
                 operator-adjudicated"
            ),
            LinkError::UnknownRelation(relation) => write!(
                f,
                "platform link: unknown relation {:?}; a connector may link only registered relations",
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

impl Platform<'_> {
    /// Deliver a single inbound message and run the agent's response cycle — a convenience for the
    /// common single-message case, equivalent to [`route_messages`](Self::route_messages) with a
    /// one-element batch.
    pub async fn route_message(
        &self,
        model: &dyn ModelClient,
        locator: &ConversationLocator,
        sender: &PersonId,
        text: &str,
        present: &[PersonId],
    ) -> Result<PlatformResponse, InstanceError> {
        self.route_messages(
            model,
            locator,
            &[MessageInput {
                sender: sender.clone(),
                text: text.to_owned(),
            }],
            present,
        )
        .await
    }

    /// Deliver a batch of inbound messages and run one agent response cycle. The client hands over
    /// the room it arrived in, the messages (each with its own sender), and who is currently present
    /// (as platform user ids); the server resolves them to stubs (minting on first contact), opens or
    /// continues a session — freezing a fresh brief at each open — appends each inbound turn, runs
    /// the loop once, and returns the outcome.
    pub async fn route_messages(
        &self,
        model: &dyn ModelClient,
        locator: &ConversationLocator,
        messages: &[MessageInput],
        present: &[PersonId],
    ) -> Result<PlatformResponse, InstanceError> {
        // Hold a stream permit for this batch's whole handling — the turn and any compaction flush
        // it triggers — so no more than `max_concurrent_streams` messages crowd the shared model at
        // once (spec §Concurrency). Released when this scope returns.
        let _stream = self
            .server
            .streams
            .acquire()
            .await
            .expect("the stream semaphore is never closed");

        // Resolve the room (minting its context memory on first contact), then the participants.
        // Each resolution borrows the store, clock, and graph fields disjointly and releases before
        // the next, so the interleaved `materialize_from` calls are free to take the graph mutably.
        let conversation = self.ensure_conversation(locator)?;

        // The unique platform ids to resolve: everyone present, plus every sender. Deduplicating
        // matters because resolution reads the graph, which is not re-materialized between mints
        // within this call — the same id seen twice would otherwise be minted twice.
        let mut uids: Vec<&PersonId> = Vec::new();
        for person in present.iter().chain(messages.iter().map(|m| &m.sender)) {
            if !uids.contains(&person) {
                uids.push(person);
            }
        }
        let mut present_set = Vec::new();
        let mut participant_ids: HashMap<&PersonId, MemoryId> = HashMap::new();
        for person in &uids {
            let id = {
                // Graph before store, per the lock-ordering rule.
                let graph = self.server.engine.graph.lock();
                resolve_or_mint_participant(
                    self.server.engine.store.lock().as_mut(),
                    self.server.engine.clock.as_ref(),
                    &graph,
                    person.platform.as_str(),
                    person.id.as_str(),
                )?
            };
            participant_ids.insert(*person, id);
            present_set.push(id);
        }
        self.server
            .engine
            .graph
            .lock()
            .materialize_from(self.server.engine.store.lock().as_ref())?;

        // Build the inbound batch and generate turn ids. The participant turns are recorded inside
        // `run_session_turn` (after `ensure_session` opens the session) so the session's `start_seq`
        // precedes them — the flush substance gate reads the buffer from `start_seq`, and must see the
        // turns to measure their delta.
        let mut inbound: Vec<InboundMessage> = Vec::with_capacity(messages.len());
        let mut participant_turn_ids: Vec<TurnId> = Vec::with_capacity(messages.len());
        for msg in messages {
            let participant = *participant_ids.get(&msg.sender).unwrap();
            let turn_id = TurnId::generate();
            inbound.push(InboundMessage {
                participant,
                text: msg.text.clone(),
            });
            participant_turn_ids.push(turn_id);
        }

        // Open or continue the session and run the turn under ordinary platform authority.
        let (report, buffer) = self
            .server
            .run_session_turn(
                model,
                &RoutedTurn {
                    conversation,
                    present_set: &present_set,
                    inbound: &inbound,
                    participant_turn_ids: &participant_turn_ids,
                    template: PromptTemplateName::Scaffold,
                    authority: Authority::Platform,
                },
            )
            .await?;

        // A deferred turn skips the compaction check entirely: the model just proved unreachable,
        // so the pre-compaction flush could not run anyway, and the buffer gained no agent turn.
        if report.outcome == TurnOutcome::Deferred {
            return Ok(PlatformResponse {
                outcome: report.outcome,
                participant_turn_ids: report
                    .participant_turn_ids
                    .iter()
                    .map(|id| id.0.to_string())
                    .collect(),
            });
        }

        // Token-triggered compaction: if the turn's peak prompt crossed the budget, end the session
        // now so the next message re-segments with a fresh brief and a carried tail (spec
        // §Compaction). The estimate fallback keeps the trigger meaningful when the backend reports
        // no usage (the in-memory and no-openai builds).
        let token_budget = Settings::from_store(self.server.engine.store.lock().as_ref())?
            .compaction
            .token_budget;
        let observed = report
            .prompt_tokens
            .map(i64::from)
            .unwrap_or_else(|| estimate_tokens(&buffer, messages));
        // `reported` distinguishes the authoritative real-usage path from the coarse estimate
        // fallback: if the backend never reports `prompt_tokens`, the trigger is running on the
        // (system-prefix-omitting) estimate, which is an operability signal worth seeing.
        tracing::debug!(
            observed,
            token_budget,
            reported = report.prompt_tokens.is_some(),
            "compaction trigger check",
        );
        if observed > token_budget
            && let Err(error) = self.end_session_for_compaction(conversation, model).await
        {
            // The turn's outcome is already in hand; if the model went down between the reply and
            // the compaction flush, deliver the reply rather than turning it into an error. The
            // flush failed before `SessionEnded`, so the session is still open in the log — the
            // next message's cold-start recovery resumes or closes it (the session was already
            // taken out of the live map).
            match &error {
                InstanceError::Turn {
                    error: TurnError::Model(model_error),
                    ..
                } if model_error.is_unavailable() => {
                    tracing::warn!(
                        %error,
                        "the model became unreachable during the compaction flush; delivering \
                         the reply and leaving the session for recovery"
                    );
                }
                _ => return Err(error),
            }
        }
        Ok(PlatformResponse {
            outcome: report.outcome,
            participant_turn_ids: report
                .participant_turn_ids
                .iter()
                .map(|id| id.0.to_string())
                .collect(),
        })
    }

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

        let joiner = {
            // Graph before store, per the lock-ordering rule.
            let graph = self.server.engine.graph.lock();
            resolve_or_mint_participant(
                self.server.engine.store.lock().as_mut(),
                self.server.engine.clock.as_ref(),
                &graph,
                participant.platform.as_str(),
                participant.id.as_str(),
            )?
        };
        self.server
            .engine
            .graph
            .lock()
            .materialize_from(self.server.engine.store.lock().as_ref())?;

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

        // Resolve the roster to memory ids, deduplicating first: resolution reads the graph, which is
        // not re-materialized between mints within this pass, so the same id seen twice would
        // otherwise be minted twice.
        let mut uids: Vec<&PersonId> = Vec::new();
        for person in roster {
            if !uids.contains(&person) {
                uids.push(person);
            }
        }
        let mut present_ids = Vec::with_capacity(uids.len());
        for person in &uids {
            let id = {
                // Graph before store, per the lock-ordering rule.
                let graph = self.server.engine.graph.lock();
                resolve_or_mint_participant(
                    self.server.engine.store.lock().as_mut(),
                    self.server.engine.clock.as_ref(),
                    &graph,
                    person.platform.as_str(),
                    person.id.as_str(),
                )?
            };
            present_ids.push(id);
        }
        self.server
            .engine
            .graph
            .lock()
            .materialize_from(self.server.engine.store.lock().as_ref())?;

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

    /// Write context entries to a conversation's context memory under platform authority. A
    /// connector (e.g. the Discord bot) uses this to write channel metadata and laconic guidance on
    /// first contact, posting structured data rather than interpolating untrusted strings into code.
    ///
    /// The context memory is resolved (or minted) by name from the locator's scope — independent of any
    /// conversation, so a connector can establish context for a scope that has no messages of its own (a
    /// guild), and can establish a room's context before its first participant message. A room's first
    /// message reuses the same memory by name. Each entry is appended as `Public` under the agent's
    /// teller. The `max_entry_chars` guard is bypassed (passed as `usize::MAX`): platform-authority
    /// context writes are blessed, like self-memories, and not subject to the agent's entry length limit.
    pub fn write_context(
        &self,
        locator: &ConversationLocator,
        connector_id: &str,
        entries: &[ContextEntry],
    ) -> Result<(), InstanceError> {
        if entries.is_empty() {
            return Ok(());
        }
        // Resolve (or mint) the scope's context memory by name — no conversation, so this works for a
        // guild as well as a room. We materialize so the append sees a freshly minted one.
        let engine = &self.server.engine;
        let context_memory = {
            let graph = engine.graph.lock();
            resolve_or_mint_context(
                engine.store.lock().as_mut(),
                engine.clock.as_ref(),
                &graph,
                locator,
            )?
        };
        engine
            .graph
            .lock()
            .materialize_from(engine.store.lock().as_ref())?;

        let mut block = MemoryBlock::new(
            engine.clone(),
            Teller::Agent,
            Authority::Platform,
            None,
            None,
            Vec::new(),
            usize::MAX,
        )?;
        for entry in entries {
            let opts = AppendOptions {
                visibility: Some(VisibilityChoice::Public),
                ..AppendOptions::default()
            };
            block
                .append(context_memory, &entry.text, opts)
                .map_err(InstanceError::Memory)?;
        }
        let now = engine.clock.now();
        engine.store.lock().append(
            now,
            EventSource::Connector(connector_id.to_owned()),
            block.into_effects().events,
        )?;
        engine
            .graph
            .lock()
            .materialize_from(engine.store.lock().as_ref())?;
        Ok(())
    }

    /// Project platform attributes onto a scoped memory as ordinary `Public` entries: a participant's
    /// current handles onto their `person/*` stub, or a guild's name onto its `context/*` memory. Each
    /// attribute either records a new value, superseding the entry a prior projection returned for it, or
    /// clears a value no longer set, retracting that entry. The connector holds the entry ids, so the
    /// server keys nothing itself.
    ///
    /// The target is resolved (or minted), so a projection lands even on first contact. Returns the new
    /// entry id per attribute, in request order: `Some` for a recorded value, `None` for a cleared or
    /// absent one. A supersede or retract target the agent has since dropped is a no-op — the fresh
    /// append still stands — so a projection never fails on a target that moved underneath it.
    pub fn project(
        &self,
        target: &LinkNode,
        connector_id: &str,
        attributes: &[ParticipantAttribute],
    ) -> Result<Vec<Option<EntryId>>, InstanceError> {
        if attributes.is_empty() {
            return Ok(Vec::new());
        }
        let engine = &self.server.engine;
        // Resolve (or mint) the target memory, the same path a message or a link takes.
        let memory = self.resolve_or_mint_node(target)?;
        engine
            .graph
            .lock()
            .materialize_from(engine.store.lock().as_ref())?;

        // No conversation to attribute to — a projection is about the subject, not a room.
        let mut block = MemoryBlock::new(
            engine.clone(),
            Teller::Agent,
            Authority::Platform,
            None,
            None,
            Vec::new(),
            usize::MAX,
        )?;
        let mut results = Vec::with_capacity(attributes.len());
        for attribute in attributes {
            match &attribute.text {
                Some(text) => {
                    let opts = AppendOptions {
                        visibility: Some(VisibilityChoice::Public),
                        ..AppendOptions::default()
                    };
                    let new = block
                        .append(memory, text, opts)
                        .map_err(InstanceError::Memory)?;
                    if let Some(old) = attribute.supersedes {
                        supersede_if_live(&mut block, memory, old, new)?;
                    }
                    results.push(Some(new));
                }
                None => {
                    if let Some(old) = attribute.supersedes {
                        retract_if_live(&mut block, memory, old)?;
                    }
                    results.push(None);
                }
            }
        }
        let now = engine.clock.now();
        engine.store.lock().append(
            now,
            EventSource::Connector(connector_id.to_owned()),
            block.into_effects().events,
        )?;
        engine
            .graph
            .lock()
            .materialize_from(engine.store.lock().as_ref())?;
        Ok(results)
    }

    /// Assert (or, with `remove`, retract) a structural link a connector authored between two of its
    /// own scoped memories — a channel's or a participant's placement in a guild, say. Both endpoints
    /// are named under the connector's platform, so a connector can only ever link memories it owns.
    /// The edge is `Public` (a structural fact, not a told aside) and carries
    /// [`LinkSource::Connector`], so an audit reads which connector authored it. `same_as` is refused:
    /// cross-platform identity is operator-adjudicated, never a connector's to assert.
    ///
    /// On assert, each endpoint is resolved or minted, so a link lands even on first sight of the guild
    /// or member. On retract, the endpoints are resolved without minting — an edge to a node that does
    /// not exist cannot exist, so the retract is a no-op rather than a pointless mint.
    pub fn link(
        &self,
        from: &LinkNode,
        to: &LinkNode,
        relation: &str,
        connector_id: &str,
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
            let from_id = self.resolve_or_mint_node(from)?;
            let to_id = self.resolve_or_mint_node(to)?;
            // Materialize the freshly minted endpoints so the edge apply resolves their classes.
            engine
                .graph
                .lock()
                .materialize_from(engine.store.lock().as_ref())?;
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
                    source: LinkSource::Connector(connector_id.to_owned()),
                    // No teller and no told_in: a connector's structural edge has no human behind it,
                    // mirroring the adjudication pass's authored `same_as`.
                    told_by: None,
                    told_in: None,
                    visibility: Visibility::Public,
                },
            )
        };
        let now = engine.clock.now();
        engine.store.lock().append(
            now,
            EventSource::Connector(connector_id.to_owned()),
            vec![payload],
        )?;
        engine
            .graph
            .lock()
            .materialize_from(engine.store.lock().as_ref())?;
        Ok(())
    }

    /// Resolve a link endpoint to its memory id, minting one on first contact — the assert path, where
    /// a guild or member seen for the first time should still take the edge.
    fn resolve_or_mint_node(&self, node: &LinkNode) -> Result<MemoryId, InstanceError> {
        let engine = &self.server.engine;
        let graph = engine.graph.lock();
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

    /// Force the live session in `locator`'s room to end and compact right now, through the exact path
    /// the token-budget trigger drives — the pre-compaction flush, the raw-transcript and working-set
    /// carryover staging, and a fresh session seeded from that carryover on the next message. This
    /// states the intent "cut here" directly, so a caller that wants a compaction seam at a chosen
    /// point does not have to size a token budget so the organic trigger *happens* to fire. Returns
    /// whether a live session was compacted — `false` if the room has never been seen or has no live
    /// session.
    pub async fn force_compaction(
        &self,
        model: &dyn ModelClient,
        locator: &ConversationLocator,
    ) -> Result<bool, InstanceError> {
        let Some(conversation) = self
            .server
            .engine
            .graph
            .lock()
            .conversation_for_locator(locator)?
        else {
            return Ok(false);
        };
        if self.server.sessions.get(conversation).is_none() {
            return Ok(false);
        }
        self.end_session_for_compaction(conversation, model).await?;
        Ok(true)
    }

    /// End the live session because the buffer crossed the token budget (spec §Compaction). Runs the
    /// budget-gated pre-compaction flush and records `SessionEnded` with a [`SessionEndCause::Compaction`]
    /// cause (inside [`Instance::flush_and_end`]). Nothing is staged for the reopen: the next message's
    /// `ensure_session` reconstructs the tail from the log, and because it reopens promptly (within the
    /// idle gap) it reads as a warm continuation and carries this session's touched set — no runtime hand-
    /// off needed (issue #86).
    async fn end_session_for_compaction(
        &self,
        conversation: ConversationId,
        model: &dyn ModelClient,
    ) -> Result<(), InstanceError> {
        // Take the session out under the map guard; the `Arc` then carries it across the flush and
        // `shutdown_mcp().await` inside `flush_and_end` without holding the guard.
        let Some(open) = self.server.sessions.remove(conversation) else {
            return Ok(());
        };
        // Flush the ending session's working state to memory and record `SessionEnded`; the buffer the
        // flush reads includes the turn that crossed the budget.
        let flushed = self
            .server
            .flush_and_end(
                conversation,
                open.as_ref(),
                model,
                SessionEndCause::Compaction,
            )
            .await?;
        tracing::info!(
            ?conversation,
            session = ?open.id,
            flushed,
            "token budget crossed; ended session for compaction",
        );
        crate::metrics::observe_compaction();
        Ok(())
    }
}

/// Supersede `old` by `new` on `memory`, treating an `old` the agent has already dropped as a no-op.
/// A projection's supersede target can vanish between projections — the agent may have retracted or
/// superseded it — so a missing target leaves the fresh append standing rather than failing the write.
fn supersede_if_live(
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
fn retract_if_live(
    block: &mut MemoryBlock,
    memory: MemoryId,
    old: EntryId,
) -> Result<(), InstanceError> {
    match block.retract(memory, old, "no longer set on the platform.") {
        Ok(()) | Err(MemoryError::UnknownEntry(_)) => Ok(()),
        Err(e) => Err(InstanceError::Memory(e)),
    }
}

/// A deterministic `chars / 4` estimate of the prompt's token count over the buffer plus the inbound
/// message — the compaction-trigger fallback when the backend reports no usage. Coarse and an
/// under-count (it omits the frozen prefix); only the real client's `prompt_tokens` is authoritative.
fn estimate_tokens(buffer: &[TurnView], messages: &[MessageInput]) -> i64 {
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
mod tests {
    use super::*;
    use crate::{
        TEST_PLATFORM,
        event::TurnRole,
        ids::{Seq, TurnId},
        time::Timestamp,
    };

    fn turn(seq: u64, text: &str) -> TurnView {
        TurnView {
            seq: Seq(seq),
            turn_id: TurnId::generate(),
            role: TurnRole::Participant,
            text: text.to_owned(),
            participant: None,
            recorded_at: Timestamp::from_millis(0),
            steps: Vec::new(),
            produced_by: None,
        }
    }

    #[test]
    fn estimate_tokens_counts_buffer_and_messages() {
        let buffer = vec![turn(1, "12345678")]; // 8 chars
        // (8 + 4) / 4 = 3.
        let messages = vec![MessageInput {
            sender: PersonId::new(TEST_PLATFORM, "dave"),
            text: "1234".to_owned(),
        }];
        assert_eq!(estimate_tokens(&buffer, &messages), 3);
    }
}
