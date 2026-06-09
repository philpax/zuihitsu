//! The agent server: the single writer that owns the event log, the materialized graph, and the
//! clock, and exposes its API split by client authority (spec §Clients and the server boundary).
//!
//! Authority is a property of the client's role, enforced here — never of where the client runs.
//! The operator-authority surface is [`Control`] (agent creation and read-only inspection; its
//! writes are authored as source `Debugger`). The platform-authority surface — delivering
//! participant turns via `route_message` — arrives with the agent loop in Stage 4 as a sibling
//! facet that structurally lacks Control's creation and inspection methods, which is what makes
//! "the operator has no platform identity" enforceable.

mod control;
#[cfg(feature = "lua")]
mod platform;

pub use control::{Arbitration, Control};
#[cfg(feature = "lua")]
pub use platform::Platform;

use std::sync::Arc;

use crate::{
    agent::genesis::{self, GenesisStatus},
    clock::Clock,
    engine::Engine,
    graph::{Graph, GraphError},
    store::{MemoryStore, Store, StoreError},
};
#[cfg(feature = "lua")]
use crate::{
    agent::lua::Session,
    agent::{Turn, TurnError, TurnReport, TurnView, buffer_turns, run_turn},
    event::{EventPayload, Initiation, PromptTemplateName, TurnRole},
    ids::{ConversationId, MemoryId, MemoryName, Seq, SessionId, TurnId},
    memory::{
        brief::{self, BriefError},
        identity::IdentityError,
        memory_block::Authority,
        scheduler::{self, SchedulerError},
    },
    model::ModelClient,
    settings::Settings,
};
#[cfg(feature = "mcp")]
use std::collections::BTreeMap;
#[cfg(feature = "lua")]
use std::collections::HashMap;

#[cfg(feature = "mcp")]
use crate::{
    agent::McpCatalogue,
    mcp::{McpHost, McpServerConfig},
};

pub struct Server {
    // The store, graph, and clock bundled behind one shared [`Engine`], so a turn shares them with a
    // single pointer bump and the Lua block API can hold a `'static` handle across `eval_async`. The
    // server is still the single writer; the engine's mutexes serialize access rather than admit a
    // second writer. See [`Engine`] for the graph-before-store lock-ordering rule.
    engine: Arc<Engine>,
    /// The live session per conversation: its id, the VM whose globals persist across the session's
    /// turns, the frozen brief, and the last-activity time the idle-gap is measured from. Pure
    /// runtime state — never logged (the `SessionStarted` / `SessionEnded` events are); an agent
    /// restart drops it and the next message opens a fresh session.
    #[cfg(feature = "lua")]
    sessions: HashMap<ConversationId, OpenSession>,
    /// Carryover staged by a token-triggered compaction, consumed by the next `ensure_session` to
    /// seed the re-segmented session (spec §Compaction). Keyed by conversation; an entry lives only
    /// between the compacting turn and the next message in that room.
    #[cfg(feature = "lua")]
    pending_carryover: HashMap<ConversationId, Carryover>,
    /// The MCP host and the catalogue probed from it at [`Server::connect_mcp`] — `None` until then.
    /// Each session opened while it is set gets the `mcp.<server>.*` projection over the same catalogue.
    #[cfg(feature = "mcp")]
    mcp: Option<McpRuntime>,
}

/// The connected MCP runtime: the host that spawns server instances and the catalogue probed from it
/// once at startup (shared into every session opened thereafter).
#[cfg(feature = "mcp")]
struct McpRuntime {
    host: Arc<dyn McpHost>,
    catalogue: McpCatalogue,
}

impl Server {
    pub fn new(store: Box<dyn Store>, graph: Graph, clock: Box<dyn Clock>) -> Server {
        Server {
            engine: Engine::new(store, graph, clock),
            #[cfg(feature = "lua")]
            sessions: HashMap::new(),
            #[cfg(feature = "lua")]
            pending_carryover: HashMap::new(),
            #[cfg(feature = "mcp")]
            mcp: None,
        }
    }

    /// Connect the configured MCP servers: probe each one's tool catalogue once through `host` (spec
    /// §startup probe), then project that catalogue into every session opened from now on. Called once
    /// after construction by whoever drives serving. A probe-level hard error (a stale `allow`/`deny`,
    /// a duplicate escaped tool name) is surfaced; a server that simply fails to spawn is dropped.
    #[cfg(feature = "mcp")]
    pub async fn connect_mcp(
        &mut self,
        host: Arc<dyn McpHost>,
        configs: BTreeMap<String, McpServerConfig>,
    ) -> Result<(), ServerError> {
        let catalogue = McpCatalogue::probe(host.as_ref(), &configs).await?;
        self.mcp = Some(McpRuntime { host, catalogue });
        Ok(())
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
        let applied = self
            .engine
            .graph
            .lock()
            .materialize_from(self.engine.store.lock().as_ref())?;
        let status = genesis::status(self.engine.store.lock().as_ref())?;
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

/// One routed turn's inputs: the `conversation` it lands in, who is `present_set` (for the session
/// brief), the `participant` it is attributed to, the `inbound` text, and the `template`/`authority`
/// that frame it — `Scaffold`/`Platform` for an ordinary message, `Imprint`/`Operator` for the
/// control-panel interview. Bundled so [`Server::run_session_turn`] takes the routed turn as a whole.
#[cfg(feature = "lua")]
struct RoutedTurn<'a> {
    conversation: ConversationId,
    present_set: &'a [MemoryId],
    participant: MemoryId,
    inbound: &'a str,
    template: PromptTemplateName,
    authority: Authority,
}

/// The session machinery shared by both facets: opening/continuing a session and running one turn.
/// On `Server` (not a facet) so the platform `route_message` and the operator `imprint` both reach
/// it.
#[cfg(feature = "lua")]
impl Server {
    /// Open or continue the session for `conversation`, then run one turn of `inbound` from
    /// `participant` under `template`/`authority`, returning its report and the live buffer it saw
    /// (the buffer the caller's compaction trigger measures). The shared core behind
    /// `Platform::route_message` and `Control::imprint`.
    async fn run_session_turn(
        &mut self,
        model: &dyn ModelClient,
        routed: &RoutedTurn<'_>,
    ) -> Result<(TurnReport, Vec<TurnView>), ServerError> {
        self.ensure_session(routed.conversation, routed.present_set)
            .await?;
        let max_steps = Settings::from_store(self.engine.store.lock().as_ref())?
            .turn
            .max_steps as usize;
        let open = self
            .sessions
            .get(&routed.conversation)
            .expect("ensure_session left an open session");
        // The live buffer the model sees as the prompt suffix: the session's prior turns (or, across
        // a compaction seam, the carried tail plus this session's turns), read from `start_seq`.
        let buffer = buffer_turns(
            self.engine.store.lock().as_ref(),
            routed.conversation,
            open.start_seq,
        )?;
        let report = run_turn(Turn {
            session: &open.vm,
            model,
            engine: self.engine.clone(),
            inbound: routed.inbound,
            inbound_participant: routed.participant,
            brief: &open.brief,
            buffer: &buffer,
            template: routed.template,
            authority: routed.authority,
            max_steps,
        })
        .await?;
        Ok((report, buffer))
    }

    /// Ensure a live session for `conversation`: reuse the open one if activity is within the
    /// idle-gap, otherwise end it (if any) and open a new one — composing and freezing its brief and
    /// minting a fresh VM. The session boundary is recorded (`SessionStarted` / `SessionEnded`) and
    /// not recomputed at replay.
    async fn ensure_session(
        &mut self,
        conversation: ConversationId,
        present_set: &[MemoryId],
    ) -> Result<(), ServerError> {
        let now = self.engine.clock.now();
        let settings = Settings::from_store(self.engine.store.lock().as_ref())?;
        let idle_gap_ms = settings.compaction.idle_gap_seconds.saturating_mul(1_000);

        let reuse = self
            .sessions
            .get(&conversation)
            .is_some_and(|open| now.as_millis() - open.last_activity.as_millis() <= idle_gap_ms);
        if reuse {
            if let Some(open) = self.sessions.get_mut(&conversation) {
                open.last_activity = now;
            }
            return Ok(());
        }

        // Catch the wake-up scheduler up to now before the session opens. This is global (it fires
        // every due trigger, not just this conversation's) and stands in for the background driver that
        // runs `fire_due` on a timer once the runtime host exists (deferred to Stage 10, spec
        // §Scheduled work). Firing here, before the drain below, is what lets a just-due item surface
        // in this session if it is eligible.
        let fired = {
            // Two guards at once: graph before store, per the lock-ordering rule.
            let graph = self.engine.graph.lock();
            scheduler::fire_due(self.engine.store.lock().as_mut(), &graph, now)?
        };
        if fired > 0 {
            self.engine
                .graph
                .lock()
                .materialize_from(self.engine.store.lock().as_ref())?;
        }

        // A lapsed session ends before the new one opens: tear down its MCP instances, then record
        // the boundary.
        if let Some(old) = self.sessions.remove(&conversation) {
            #[cfg(feature = "mcp")]
            old.vm.shutdown_mcp().await;
            self.engine.store.lock().append(
                now,
                vec![EventPayload::SessionEnded {
                    conversation,
                    id: old.id,
                }],
            )?;
        }

        // A pending carryover from a just-compacted session seeds the new one: the next buffer read
        // starts at the carried tail (not this `SessionStarted`), the boundary is recorded as
        // `seeded_from_turn` for faithful replay, and the touch-derived working set augments the new
        // brief as active threads (spec §Compaction → carryover).
        let carryover = self.pending_carryover.remove(&conversation);
        let seeded_from_turn = carryover.as_ref().map(|carry| carry.seeded_from_turn);
        let working_set: &[MemoryId] = carryover
            .as_ref()
            .map_or(&[], |carry| carry.working_set.as_slice());

        let context = self
            .engine
            .graph
            .lock()
            .context_for_conversation(conversation)?;
        let brief = brief::compose(
            &self.engine.graph.lock(),
            &settings.brief,
            &brief::BriefRequest {
                present_set,
                current_context: context,
                working_set,
                now,
            },
        )?;
        let id = SessionId::generate();
        let committed = self.engine.store.lock().append(
            now,
            vec![EventPayload::SessionStarted {
                conversation,
                id,
                participants: present_set.to_vec(),
                started_at: now,
                seeded_from_turn,
                brief: brief.clone(),
            }],
        )?;
        let session_start_seq = committed[0].seq;
        self.engine
            .graph
            .lock()
            .materialize_from(self.engine.store.lock().as_ref())?;
        // The VM carries the MCP projection when servers are connected; otherwise a plain VM.
        #[cfg(feature = "mcp")]
        let vm = match &self.mcp {
            Some(runtime) => Session::with_mcp(
                conversation,
                runtime.host.clone(),
                runtime.catalogue.clone(),
            ),
            None => Session::new(conversation),
        };
        #[cfg(not(feature = "mcp"))]
        let vm = Session::new(conversation);
        self.sessions.insert(
            conversation,
            OpenSession {
                id,
                vm,
                brief,
                last_activity: now,
                start_seq: carryover
                    .map(|carry| carry.from_seq)
                    .unwrap_or(session_start_seq),
            },
        );

        // Drain the wake-up surface into the opening session: fired items that are both visible to and
        // targeted at this present set are raised as one `Initiated` system turn the agent sees in its
        // buffer, and each is marked surfaced so it is never raised again (spec §Agent-initiated
        // speech). Appended after `SessionStarted`, so it falls inside the buffer read from `start_seq`.
        // Bind the drain result so the graph guard from the scrutinee is released before the body
        // re-locks the graph below (the lock is not reentrant).
        let drained =
            scheduler::drain(&self.engine.graph.lock(), present_set, &settings.scheduler)?;
        if let Some(drained) = drained {
            let turn_id = TurnId::generate();
            let mut payloads = vec![EventPayload::ConversationTurn {
                conversation,
                turn_id,
                role: TurnRole::System,
                text: drained.text,
                participant: None,
                initiation: Initiation::Initiated,
                produced_by: None,
            }];
            for (entry_id, memory) in drained.entries {
                payloads.push(EventPayload::ScheduledItemSurfaced {
                    entry_id,
                    memory,
                    session: id,
                    surfaced_at: now,
                });
            }
            self.engine.store.lock().append(now, payloads)?;
            self.engine
                .graph
                .lock()
                .materialize_from(self.engine.store.lock().as_ref())?;
        }
        Ok(())
    }

    /// Resolve the control-panel operator's stable `person/operator` stub, minting it once on the
    /// first imprint. Unlike a platform participant it carries no `ParticipantIdentified` binding —
    /// the operator has no platform identity, must never collide with a real participant, and must
    /// resolve identically across imprints — so it is keyed only by its canonical name.
    fn resolve_or_mint_operator(&mut self) -> Result<MemoryId, ServerError> {
        if let Some(memory) = self.engine.graph.lock().memory_by_name("person/operator")? {
            return Ok(memory.id);
        }
        let id = MemoryId::generate();
        let now = self.engine.clock.now();
        self.engine.store.lock().append(
            now,
            vec![EventPayload::MemoryCreated {
                id,
                name: MemoryName::new("person/operator"),
            }],
        )?;
        self.engine
            .graph
            .lock()
            .materialize_from(self.engine.store.lock().as_ref())?;
        Ok(id)
    }
}

/// The raw-transcript carryover a compaction stages for the next session (spec §Compaction →
/// raw-transcript carryover). The oldest carried turn is both the `seeded_from_turn` boundary
/// recorded on the new `SessionStarted` and the `from_seq` the new session's buffer is read from, so
/// the carried tail plus the new turns reconstruct the post-cut buffer.
#[cfg(feature = "lua")]
struct Carryover {
    seeded_from_turn: TurnId,
    from_seq: Seq,
    /// The memories the ending session touched (read or wrote), re-surfaced in the new session's
    /// brief as active threads — the touch-derived working set (spec §Compaction → working-set
    /// carryover).
    working_set: Vec<MemoryId>,
}

/// The live session backing a conversation (runtime state, see [`Server::sessions`]).
#[cfg(feature = "lua")]
struct OpenSession {
    id: SessionId,
    vm: Session,
    brief: String,
    last_activity: crate::time::Timestamp,
    /// The log seq the live buffer is read from: the `SessionStarted` seq for a fresh or idle-opened
    /// session, or a carried tail's seq across a compaction seam (so the carryover plus this
    /// session's turns reconstruct the buffer — see [`buffer_turns`]).
    start_seq: Seq,
}

/// A server-side failure, delegating its message to the underlying error.
#[derive(Debug)]
pub enum ServerError {
    Store(StoreError),
    Graph(GraphError),
    /// A turn (the agent loop) failed while routing a message.
    #[cfg(feature = "lua")]
    Turn(TurnError),
    /// Connecting the MCP servers failed (a probe-level hard error, e.g. a stale `allow`/`deny`).
    #[cfg(feature = "mcp")]
    Mcp(crate::mcp::McpError),
}

impl std::fmt::Display for ServerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ServerError::Store(error) => write!(f, "server (store): {error}"),
            ServerError::Graph(error) => write!(f, "server (graph): {error}"),
            #[cfg(feature = "lua")]
            ServerError::Turn(error) => write!(f, "server (turn): {error}"),
            #[cfg(feature = "mcp")]
            ServerError::Mcp(error) => write!(f, "server (mcp): {error}"),
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
            #[cfg(feature = "mcp")]
            ServerError::Mcp(error) => Some(error),
        }
    }
}

#[cfg(feature = "mcp")]
impl From<crate::mcp::McpError> for ServerError {
    fn from(error: crate::mcp::McpError) -> Self {
        ServerError::Mcp(error)
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
impl From<SchedulerError> for ServerError {
    fn from(error: SchedulerError) -> Self {
        match error {
            SchedulerError::Store(error) => ServerError::Store(error),
            SchedulerError::Graph(error) => ServerError::Graph(error),
        }
    }
}

#[cfg(feature = "lua")]
impl From<TurnError> for ServerError {
    fn from(error: TurnError) -> Self {
        ServerError::Turn(error)
    }
}
