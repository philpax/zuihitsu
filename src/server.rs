//! The agent server: the single writer that owns the event log, the materialized graph, and the
//! clock, and exposes its API split by client authority (spec §Clients and the server boundary).
//!
//! Authority is a property of the client's role, enforced here — never of where the client runs.
//! The operator-authority surface is [`Control`] (agent creation and read-only inspection; its
//! writes are authored as source `Debugger`). The platform-authority surface — delivering
//! participant turns via `route_message` — arrives with the agent loop in Stage 4 as a sibling
//! facet that structurally lacks Control's creation and inspection methods, which is what makes
//! "the operator has no platform identity" enforceable.

#[cfg(feature = "lua")]
use crate::{
    agent::{Turn, TurnError, TurnOutcome, run_turn},
    brief::{self, BriefError},
    event::EventPayload,
    identity::{IdentityError, resolve_or_mint_conversation, resolve_or_mint_participant},
    ids::{ConversationId, MemoryId, SessionId},
    lua::Session,
    model::ModelClient,
};
use crate::{
    clock::Clock,
    genesis::{self, GenesisStatus, Rollout, SeedSelf},
    graph::{Graph, GraphError, MemoryView, SessionView},
    ids::ConversationLocator,
    settings::Settings,
    store::{MemoryStore, Store, StoreError},
};
#[cfg(feature = "lua")]
use std::collections::HashMap;

pub struct Server {
    store: Box<dyn Store>,
    graph: Graph,
    clock: Box<dyn Clock>,
    /// The live session per conversation: its id, the VM whose globals persist across the session's
    /// turns, the frozen brief, and the last-activity time the idle-gap is measured from. Pure
    /// runtime state — never logged (the `SessionStarted` / `SessionEnded` events are); an agent
    /// restart drops it and the next message opens a fresh session.
    #[cfg(feature = "lua")]
    sessions: HashMap<ConversationId, OpenSession>,
}

impl Server {
    pub fn new(store: Box<dyn Store>, graph: Graph, clock: Box<dyn Clock>) -> Server {
        Server {
            store,
            graph,
            clock,
            #[cfg(feature = "lua")]
            sessions: HashMap::new(),
        }
    }

    /// A server backed entirely in memory (in-memory store and graph), for tests.
    pub fn in_memory(clock: Box<dyn Clock>) -> Result<Server, ServerError> {
        Ok(Server::new(
            Box::new(MemoryStore::new()),
            Graph::open_in_memory()?,
            clock,
        ))
    }

    /// Catch the graph up to log-head — reconciling a graph left stale or half-applied by a crash
    /// in the commit window — and classify the log for the caller to act on. The single-writer log
    /// lock is acquired when the (file-backed) store is opened, before the server is constructed.
    pub fn boot(&mut self) -> Result<GenesisStatus, ServerError> {
        let applied = self.graph.materialize_from(self.store.as_ref())?;
        let status = genesis::status(self.store.as_ref())?;
        tracing::info!(?status, applied, "server booted");
        Ok(status)
    }

    /// The operator-authority API facet.
    pub fn control(&mut self) -> Control<'_> {
        Control { server: self }
    }

    /// The platform-authority API facet — delivering participant turns. It structurally lacks
    /// Control's creation and inspection methods, which is what makes "the operator has no platform
    /// identity" enforceable.
    #[cfg(feature = "lua")]
    pub fn platform(&mut self) -> Platform<'_> {
        Platform { server: self }
    }
}

/// The live session backing a conversation (runtime state, see [`Server::sessions`]).
#[cfg(feature = "lua")]
struct OpenSession {
    id: SessionId,
    vm: Session,
    brief: String,
    last_activity: crate::ids::Timestamp,
}

/// Platform-authority operations: a client delivering participant turns. It can act only as the
/// participants it represents, and cannot reach Control's operator surface.
#[cfg(feature = "lua")]
pub struct Platform<'a> {
    server: &'a mut Server,
}

#[cfg(feature = "lua")]
impl Platform<'_> {
    /// Deliver an inbound message and run the agent's response cycle. The client hands over the room
    /// it arrived in, who sent it, and who is currently present (as platform user ids); the server
    /// resolves them to stubs (minting on first contact), opens or continues a session — freezing a
    /// fresh brief at each open — appends the inbound turn, runs the loop, and returns the outcome.
    pub async fn route_message(
        &mut self,
        model: &dyn ModelClient,
        locator: &ConversationLocator,
        sender: &str,
        text: &str,
        present: &[&str],
    ) -> Result<TurnOutcome, ServerError> {
        // Resolve the room (minting its context memory on first contact) and the participants. Each
        // call borrows the store, clock, and graph fields disjointly and releases before the next,
        // so the interleaved `materialize_from` calls are free to take the graph mutably.
        let conversation = resolve_or_mint_conversation(
            self.server.store.as_mut(),
            self.server.clock.as_ref(),
            &self.server.graph,
            locator,
        )?;
        self.server
            .graph
            .materialize_from(self.server.store.as_ref())?;

        // The unique platform ids to resolve: everyone present, plus the sender. Deduplicating
        // matters because resolution reads the graph, which is not re-materialized between mints
        // within this call — the same id seen twice would otherwise be minted twice.
        let platform = locator.platform.as_str();
        let mut uids: Vec<&str> = Vec::new();
        for uid in present.iter().chain(std::iter::once(&sender)) {
            if !uids.contains(uid) {
                uids.push(uid);
            }
        }
        let mut present_set = Vec::new();
        let mut sender_id = None;
        for uid in &uids {
            let id = resolve_or_mint_participant(
                self.server.store.as_mut(),
                self.server.clock.as_ref(),
                &self.server.graph,
                platform,
                uid,
            )?;
            if *uid == sender {
                sender_id = Some(id);
            }
            present_set.push(id);
        }
        let sender_id = sender_id.expect("the sender is among the resolved ids");
        self.server
            .graph
            .materialize_from(self.server.store.as_ref())?;

        // Open or continue the session, freezing a brief at each open.
        self.ensure_session(conversation, &present_set)?;

        let max_steps = Settings::from_store(self.server.store.as_ref())?
            .turn
            .max_steps as usize;
        let open = self
            .server
            .sessions
            .get(&conversation)
            .expect("ensure_session left an open session");
        let outcome = run_turn(Turn {
            session: &open.vm,
            model,
            store: self.server.store.as_mut(),
            graph: &mut self.server.graph,
            clock: self.server.clock.as_ref(),
            inbound: text,
            inbound_participant: sender_id,
            brief: &open.brief,
            max_steps,
        })
        .await?;
        Ok(outcome)
    }

    /// Ensure a live session for `conversation`: reuse the open one if activity is within the
    /// idle-gap, otherwise end it (if any) and open a new one — composing and freezing its brief and
    /// minting a fresh VM. The session boundary is recorded (`SessionStarted` / `SessionEnded`) and
    /// not recomputed at replay.
    fn ensure_session(
        &mut self,
        conversation: ConversationId,
        present_set: &[MemoryId],
    ) -> Result<(), ServerError> {
        let now = self.server.clock.now();
        let settings = Settings::from_store(self.server.store.as_ref())?;
        let idle_gap_ms = settings.compaction.idle_gap_seconds.saturating_mul(1_000);

        let reuse =
            self.server.sessions.get(&conversation).is_some_and(|open| {
                now.as_millis() - open.last_activity.as_millis() <= idle_gap_ms
            });
        if reuse {
            if let Some(open) = self.server.sessions.get_mut(&conversation) {
                open.last_activity = now;
            }
            return Ok(());
        }

        // A lapsed session ends before the new one opens.
        if let Some(old) = self.server.sessions.remove(&conversation) {
            self.server.store.append(
                now,
                vec![EventPayload::SessionEnded {
                    conversation,
                    id: old.id,
                }],
            )?;
        }

        let context = self.server.graph.context_for_conversation(conversation)?;
        let brief = brief::compose(&self.server.graph, present_set, context, &settings.brief)?;
        let id = SessionId::generate();
        self.server.store.append(
            now,
            vec![EventPayload::SessionStarted {
                conversation,
                id,
                participants: present_set.to_vec(),
                started_at: now,
                seeded_from_turn: None,
                brief: brief.clone(),
            }],
        )?;
        self.server
            .graph
            .materialize_from(self.server.store.as_ref())?;
        self.server.sessions.insert(
            conversation,
            OpenSession {
                id,
                vm: Session::new(conversation),
                brief,
                last_activity: now,
            },
        );
        Ok(())
    }
}

/// Operator-authority operations: agent creation and read-only inspection. A platform client can
/// never obtain one of these.
pub struct Control<'a> {
    server: &'a mut Server,
}

impl Control<'_> {
    /// Create the agent — or resume an interrupted genesis — then project the new events so reads
    /// see them. Idempotent: calling it on a born agent is a no-op.
    pub fn create_agent(&mut self, seed: &SeedSelf) -> Result<Rollout, ServerError> {
        let outcome =
            genesis::rollout(self.server.store.as_mut(), self.server.clock.as_ref(), seed)?;
        self.server
            .graph
            .materialize_from(self.server.store.as_ref())?;
        Ok(outcome)
    }

    pub fn genesis_status(&self) -> Result<GenesisStatus, ServerError> {
        Ok(genesis::status(self.server.store.as_ref())?)
    }

    /// Inspect a live memory by name (e.g. `"self"`).
    pub fn memory(&self, name: &str) -> Result<Option<MemoryView>, ServerError> {
        Ok(self.server.graph.memory_by_name(name)?)
    }

    /// The agent's current behavioral settings: the latest `ConfigSet` snapshot.
    pub fn settings(&self) -> Result<Settings, ServerError> {
        Ok(Settings::from_store(self.server.store.as_ref())?)
    }

    /// The sessions of a conversation, addressed by its locator, oldest first — operator inspection
    /// of how the conversation segmented into sessions. Empty if the room has never been seen.
    pub fn sessions(&self, locator: &ConversationLocator) -> Result<Vec<SessionView>, ServerError> {
        match self.server.graph.conversation_for_locator(locator)? {
            Some(conversation) => Ok(self.server.graph.sessions_in(conversation)?),
            None => Ok(Vec::new()),
        }
    }
}

/// A server-side failure, delegating its message to the underlying error.
#[derive(Debug)]
pub enum ServerError {
    Store(StoreError),
    Graph(GraphError),
    /// A turn (the agent loop) failed while routing a message.
    #[cfg(feature = "lua")]
    Turn(TurnError),
}

impl std::fmt::Display for ServerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ServerError::Store(error) => write!(f, "server (store): {error}"),
            ServerError::Graph(error) => write!(f, "server (graph): {error}"),
            #[cfg(feature = "lua")]
            ServerError::Turn(error) => write!(f, "server (turn): {error}"),
        }
    }
}

impl std::error::Error for ServerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ServerError::Store(error) => Some(error),
            ServerError::Graph(error) => Some(error),
            #[cfg(feature = "lua")]
            ServerError::Turn(error) => Some(error),
        }
    }
}

impl From<StoreError> for ServerError {
    fn from(error: StoreError) -> Self {
        ServerError::Store(error)
    }
}

impl From<GraphError> for ServerError {
    fn from(error: GraphError) -> Self {
        ServerError::Graph(error)
    }
}

// Identity and brief resolution fail only into store/graph errors, so they map onto the existing
// variants rather than widening the enum; the agent loop's richer `TurnError` keeps its own.
#[cfg(feature = "lua")]
impl From<IdentityError> for ServerError {
    fn from(error: IdentityError) -> Self {
        match error {
            IdentityError::Store(error) => ServerError::Store(error),
            IdentityError::Graph(error) => ServerError::Graph(error),
        }
    }
}

#[cfg(feature = "lua")]
impl From<BriefError> for ServerError {
    fn from(error: BriefError) -> Self {
        match error {
            BriefError::Graph(error) => ServerError::Graph(error),
        }
    }
}

#[cfg(feature = "lua")]
impl From<TurnError> for ServerError {
    fn from(error: TurnError) -> Self {
        ServerError::Turn(error)
    }
}
