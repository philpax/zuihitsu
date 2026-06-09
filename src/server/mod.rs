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
    settings::{ConcurrencySettings, Settings},
    time::Timestamp,
};
#[cfg(feature = "mcp")]
use std::collections::BTreeMap;
#[cfg(feature = "lua")]
use std::collections::HashMap;
#[cfg(feature = "lua")]
use std::future::Future;
#[cfg(feature = "lua")]
use std::sync::atomic::{AtomicI64, Ordering};
#[cfg(feature = "lua")]
use std::time::Duration;

#[cfg(feature = "lua")]
use parking_lot::Mutex;
#[cfg(feature = "lua")]
use tokio::sync::Semaphore;

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
    /// restart drops it and the next message opens a fresh session. Behind a `Mutex` (and each value
    /// an `Arc`) so concurrent conversations reach the map through a shared `&Server`; a turn holds
    /// its session's `Arc` across the turn `.await` without keeping the map guard.
    #[cfg(feature = "lua")]
    sessions: Mutex<HashMap<ConversationId, Arc<OpenSession>>>,
    /// Carryover staged by a token-triggered compaction, consumed by the next `ensure_session` to
    /// seed the re-segmented session (spec §Compaction). Keyed by conversation; an entry lives only
    /// between the compacting turn and the next message in that room. Behind a `Mutex` for the same
    /// shared-`&Server` reason as `sessions`.
    #[cfg(feature = "lua")]
    pending_carryover: Mutex<HashMap<ConversationId, Carryover>>,
    /// The concurrent-stream limit (spec §Concurrency): a permit is held for each in-flight inbound
    /// message's whole handling, so no more than `max_concurrent_streams` turns crowd the shared
    /// model at once; further streams queue. Sized from settings at construction (a change takes
    /// effect on restart).
    #[cfg(feature = "lua")]
    streams: Semaphore,
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
        let engine = Engine::new(store, graph, clock);
        #[cfg(feature = "lua")]
        let streams = Semaphore::new(initial_stream_permits(&engine));
        Server {
            engine,
            #[cfg(feature = "lua")]
            sessions: Mutex::new(HashMap::new()),
            #[cfg(feature = "lua")]
            pending_carryover: Mutex::new(HashMap::new()),
            #[cfg(feature = "lua")]
            streams,
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

    /// The operator-authority API facet. Takes `&self` so a shared `Arc<Server>` can hand out a facet
    /// per caller; the server's mutable runtime state lives behind its own locks.
    pub fn control(&self) -> Control<'_> {
        Control { server: self }
    }

    /// The platform-authority API facet — delivering participant turns. It structurally lacks
    /// Control's creation and inspection methods, which is what makes "the operator has no platform
    /// identity" enforceable. Takes `&self` so concurrent conversations each obtain one from a shared
    /// `Arc<Server>`.
    #[cfg(feature = "lua")]
    pub fn platform(&self) -> Platform<'_> {
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
        &self,
        model: &dyn ModelClient,
        routed: &RoutedTurn<'_>,
    ) -> Result<(TurnReport, Vec<TurnView>), ServerError> {
        // `ensure_session` returns the open session as an `Arc`, so the turn holds it across
        // `run_turn().await` without keeping the `sessions` map guard.
        let open = self
            .ensure_session(routed.conversation, routed.present_set)
            .await?;
        let turn_settings = Settings::from_store(self.engine.store.lock().as_ref())?.turn;
        let max_steps = turn_settings.max_steps as usize;
        let block_timeout = Duration::from_secs(turn_settings.block_timeout_seconds.max(0) as u64);
        let max_block_attempts = turn_settings.max_block_attempts.max(1) as u32;
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
            block_timeout,
            max_block_attempts,
        })
        .await?;
        Ok((report, buffer))
    }

    /// Ensure a live session for `conversation`: reuse the open one if activity is within the
    /// idle-gap, otherwise end it (if any) and open a new one — composing and freezing its brief and
    /// minting a fresh VM. The session boundary is recorded (`SessionStarted` / `SessionEnded`) and
    /// not recomputed at replay.
    async fn ensure_session(
        &self,
        conversation: ConversationId,
        present_set: &[MemoryId],
    ) -> Result<Arc<OpenSession>, ServerError> {
        let now = self.engine.clock.now();
        let settings = Settings::from_store(self.engine.store.lock().as_ref())?;
        let idle_gap_ms = settings.compaction.idle_gap_seconds.saturating_mul(1_000);

        // Reuse the open session if its last activity is within the idle gap, bumping it. The map
        // guard is released before returning; the returned `Arc` keeps the session alive for the turn.
        {
            let sessions = self.sessions.lock();
            if let Some(open) = sessions.get(&conversation)
                && now.as_millis() - open.last_activity_millis() <= idle_gap_ms
            {
                open.touch(now);
                return Ok(open.clone());
            }
        }

        // Catch the wake-up scheduler up to now before the session opens, so a just-due item can
        // surface in this session if it is eligible (the drain below reads the fired surface). The
        // background driver ([`Server::run_scheduler`]) fires continuously on a timer; this catch-up
        // stays for immediacy at session open and is idempotent with it.
        self.fire_due_now(now)?;

        // A lapsed session ends before the new one opens: take it out under the map guard, release
        // the guard, then tear down its MCP instances and record the boundary — no map guard is held
        // across the `shutdown_mcp().await`.
        let old = self.sessions.lock().remove(&conversation);
        if let Some(old) = old {
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
        let carryover = self.pending_carryover.lock().remove(&conversation);
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
        let open = Arc::new(OpenSession {
            id,
            vm,
            brief,
            last_activity: AtomicI64::new(now.as_millis()),
            start_seq: carryover
                .map(|carry| carry.from_seq)
                .unwrap_or(session_start_seq),
        });
        self.sessions.lock().insert(conversation, open.clone());

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
        Ok(open)
    }

    /// Resolve the control-panel operator's stable `person/operator` stub, minting it once on the
    /// first imprint. Unlike a platform participant it carries no `ParticipantIdentified` binding —
    /// the operator has no platform identity, must never collide with a real participant, and must
    /// resolve identically across imprints — so it is keyed only by its canonical name.
    fn resolve_or_mint_operator(&self) -> Result<MemoryId, ServerError> {
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

    /// Fire every globally-due wake-up as of `now` and reconcile the graph if any fired (spec §Scheduled
    /// work). Shared by the session-open catch-up and the background driver, so both fire with identical
    /// semantics — it is global (every due trigger, not one conversation's) and idempotent (a fired
    /// trigger is no longer due). Holds the graph guard before the store, per the lock-ordering rule.
    fn fire_due_now(&self, now: Timestamp) -> Result<usize, ServerError> {
        let fired = {
            let graph = self.engine.graph.lock();
            scheduler::fire_due(self.engine.store.lock().as_mut(), &graph, now)?
        };
        if fired > 0 {
            self.engine
                .graph
                .lock()
                .materialize_from(self.engine.store.lock().as_ref())?;
        }
        Ok(fired)
    }

    /// The background scheduler driver (spec §Scheduled work → the timer that runs `fire_due`
    /// continuously, deferred from Stage 9 until the shared-server model existed). Every `tick` it fires
    /// all globally-due wake-ups, so a long-idle agent's reminders fire on time instead of waiting for a
    /// session to open; the eligible subset is still *surfaced* per session by the open-time drain. Runs
    /// on the shared `Arc<Server>` until `shutdown` resolves.
    ///
    /// A fire failure is logged, never propagated — the driver is long-lived and must outlast a
    /// transient store/graph error. It holds no lock across an `.await` and never touches the per-block
    /// memory locks, so it cannot deadlock with concurrent conversation turns.
    pub async fn run_scheduler(
        self: Arc<Self>,
        tick: Duration,
        shutdown: impl Future<Output = ()>,
    ) {
        let mut interval = tokio::time::interval(tick);
        tokio::pin!(shutdown);
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    let now = self.engine.clock.now();
                    match self.fire_due_now(now) {
                        Ok(fired) if fired > 0 => {
                            tracing::debug!(fired, "scheduler driver fired wake-ups")
                        }
                        Ok(_) => {}
                        Err(error) => {
                            tracing::error!(%error, "scheduler driver: firing due wake-ups failed")
                        }
                    }
                }
                _ = &mut shutdown => break,
            }
        }
        tracing::info!("scheduler driver stopped");
    }
}

/// The initial stream-limit permit count read from settings at construction. Floors at 1 so a
/// missing, zero, or negative configuration never produces a deadlocking zero-permit semaphore; a
/// store read failure falls back to the build default with a warning.
#[cfg(feature = "lua")]
fn initial_stream_permits(engine: &Engine) -> usize {
    let configured = Settings::from_store(engine.store.lock().as_ref())
        .map(|settings| settings.concurrency.max_concurrent_streams)
        .unwrap_or_else(|error| {
            tracing::warn!(%error, "could not read the stream limit; using the build default");
            ConcurrencySettings::default().max_concurrent_streams
        });
    configured.max(1) as usize
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

/// The live session backing a conversation (runtime state, see [`Server::sessions`]). Held behind an
/// `Arc` in the `sessions` map, so a running turn keeps its session alive without the map guard; only
/// `last_activity` is mutated after open, so it is an atomic the reuse path bumps through `&self`.
#[cfg(feature = "lua")]
struct OpenSession {
    id: SessionId,
    vm: Session,
    brief: String,
    /// The last-activity wall-clock in epoch millis, the idle-gap is measured from. Atomic so the
    /// idle-reuse path can bump it through the shared `&OpenSession` without a map-wide write lock.
    last_activity: AtomicI64,
    /// The log seq the live buffer is read from: the `SessionStarted` seq for a fresh or idle-opened
    /// session, or a carried tail's seq across a compaction seam (so the carryover plus this
    /// session's turns reconstruct the buffer — see [`buffer_turns`]).
    start_seq: Seq,
}

#[cfg(feature = "lua")]
impl OpenSession {
    /// The last-activity time in epoch millis.
    fn last_activity_millis(&self) -> i64 {
        self.last_activity.load(Ordering::Relaxed)
    }

    /// Record `now` as the last activity (the idle-reuse bump).
    fn touch(&self, now: crate::time::Timestamp) {
        self.last_activity.store(now.as_millis(), Ordering::Relaxed);
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
