//! The agent server: the single writer that owns the event log, the materialized graph, and the
//! clock, and exposes its API split by client authority (spec §Clients and the server boundary).
//!
//! Authority is a property of the client's role, enforced here — never of where the client runs.
//! The operator-authority surface is [`Control`] (agent creation and read-only inspection; its
//! writes are authored as source `Operator`). The platform-authority surface — delivering
//! participant turns via `route_message` — arrives with the agent loop in Stage 4 as a sibling
//! facet that structurally lacks Control's creation and inspection methods, which is what makes
//! "the operator has no platform identity" enforceable.

mod control;
mod platform;

pub use control::{Arbitration, Control, LuaConsoleOutcome, ModelCall};
pub use platform::Platform;

use std::{
    collections::{BTreeMap, HashMap},
    future::Future,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicI64, Ordering},
    },
    time::Duration,
};

use parking_lot::Mutex;
use tokio::sync::Semaphore;

use crate::{
    agent::{
        Flush, McpCatalogue, Turn, TurnError, TurnReport, TurnView, buffer_turns,
        genesis::{self, GenesisStatus},
        lua::Session,
        run_adjudicate_catch_up, run_describe_catch_up, run_flush, run_turn,
    },
    clock::Clock,
    engine::Engine,
    event::{EventPayload, Initiation, PromptTemplateName, TurnRole},
    graph::{Graph, GraphError},
    ids::{ConversationId, MemoryId, Namespace, Seq, SessionId, TurnId},
    mcp::{McpHost, McpServerConfig},
    memory::{
        brief::{self, BriefError},
        identity::IdentityError,
        memory_block::Authority,
        scheduler::{self, SchedulerError},
        search::{SearchError, SearchHit, SearchQuery, search as rank_search},
    },
    model::{
        ModelClient,
        embed::Embedder,
        index::{IndexError, apply_batch, embed_batch},
    },
    settings::{ConcurrencySettings, Settings},
    snapshot,
    store::{MemoryStore, Store, StoreError},
    time::Timestamp,
    vector::VectorIndex,
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
    /// restart drops the map, but the next message recovers a session still open in the log through
    /// `ensure_session` (resumed within the idle gap, else closed-with-flush and reopened). Behind a
    /// `Mutex` (and each value an `Arc`) so concurrent conversations reach the map through a shared
    /// `&Server`; a turn holds its session's `Arc` across the turn `.await` without keeping the map guard.
    sessions: Mutex<HashMap<ConversationId, Arc<OpenSession>>>,
    /// Carryover staged by a token-triggered compaction, consumed by the next `ensure_session` to
    /// seed the re-segmented session (spec §Compaction). Keyed by conversation; an entry lives only
    /// between the compacting turn and the next message in that room. Behind a `Mutex` for the same
    /// shared-`&Server` reason as `sessions`.
    pending_carryover: Mutex<HashMap<ConversationId, Carryover>>,
    /// The describer's cursor: the log seq through which descriptions have been regenerated. The
    /// background describer (and the explicit `describe_catch_up`) advances it as it catches synthesized
    /// descriptions up to the log off the hot path (spec §Write path → regenerate off the hot path).
    /// In-memory; `boot` re-seeds it to log-head, treating already-written state as described — a crash
    /// mid-regen self-heals on the memory's next write rather than re-describing the whole log at boot.
    describer_cursor: Mutex<Seq>,
    /// The merge-adjudicator's cursor: the log seq through which proposed merges have been adjudicated.
    /// Its own background pass (and the explicit `adjudicate_catch_up`) advances it as it weighs pending
    /// proposals off the hot path (spec §Cross-platform identity → adjudicated merge). Re-seeded to
    /// log-head at boot, like the describer's.
    adjudicator_cursor: Mutex<Seq>,
    /// The concurrent-stream limit (spec §Concurrency): a permit is held for each in-flight inbound
    /// message's whole handling, so no more than `max_concurrent_streams` turns crowd the shared
    /// model at once; further streams queue. Sized from settings at construction (a change takes
    /// effect on restart).
    streams: Semaphore,
    /// The MCP host and the catalogue probed from it at [`Server::connect_mcp`] — `None` until then.
    /// Each session opened while it is set gets the `mcp.<server>.*` projection over the same catalogue.
    mcp: Option<McpRuntime>,
}

/// The connected MCP runtime: the host that spawns server instances and the catalogue probed from it
/// once at startup (shared into every session opened thereafter).
struct McpRuntime {
    host: Arc<dyn McpHost>,
    catalogue: McpCatalogue,
}

/// The background snapshotter's policy (spec §Snapshots): where to write, how often to check, the
/// activity gate, and retention. Assembled by the serving host from the `[snapshots]` config and
/// handed to [`Server::run_snapshotter`].
pub struct SnapshotSchedule {
    pub dir: PathBuf,
    pub check_interval: Duration,
    pub min_new_events: u64,
    pub keep: usize,
}

impl Server {
    pub fn new(store: Box<dyn Store>, graph: Graph, clock: Box<dyn Clock>) -> Server {
        Server::from_engine(Engine::new(store, graph, clock))
    }

    /// As [`Server::new`], with the semantic-retrieval backends attached — the live server's
    /// configuration when an embedding endpoint is set, so `memory.search` and the background indexer
    /// have an embedder and a vector index to work over.
    pub fn with_retrieval(
        store: Box<dyn Store>,
        graph: Graph,
        clock: Box<dyn Clock>,
        embedder: Arc<dyn Embedder>,
        vectors: Box<dyn VectorIndex>,
    ) -> Server {
        Server::from_engine(Engine::with_retrieval(
            store, graph, clock, embedder, vectors,
        ))
    }

    fn from_engine(engine: Arc<Engine>) -> Server {
        let streams = Semaphore::new(initial_stream_permits(&engine));
        Server {
            engine,
            sessions: Mutex::new(HashMap::new()),
            pending_carryover: Mutex::new(HashMap::new()),
            describer_cursor: Mutex::new(Seq::ZERO),
            adjudicator_cursor: Mutex::new(Seq::ZERO),
            streams,
            mcp: None,
        }
    }

    /// Connect the configured MCP servers: probe each one's tool catalogue once through `host` (spec
    /// §startup probe), then project that catalogue into every session opened from now on. Called once
    /// after construction by whoever drives serving. A probe-level hard error (a stale `allow`/`deny`,
    /// a duplicate escaped tool name) is surfaced; a server that simply fails to spawn is dropped.
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
        // Seed the describer's cursor to log-head: state written before this boot is treated as already
        // described, so a restart does not re-describe the whole log (spec §Write path). New writes from
        // here are caught up off the hot path.
        self.baseline_describer_cursor()?;
        self.baseline_adjudicator_cursor()?;
        let status = genesis::status(self.engine.store.lock().as_ref())?;
        tracing::info!(?status, applied, "server booted");
        Ok(status)
    }

    /// Write a graph snapshot into `dir` and return its path, or `None` when the graph is already
    /// snapshotted at its current head (no events since the last one — nothing to checkpoint). Holds
    /// the graph lock across the `VACUUM INTO`, so the capture is at a clean `seq` boundary: a commit,
    /// which takes the same lock, can neither be in flight nor interleave (spec §Snapshots). Creates
    /// `dir` if absent.
    pub fn snapshot(&self, dir: &Path) -> Result<Option<PathBuf>, ServerError> {
        let graph = self.engine.graph.lock();
        let head = graph.head()?;
        std::fs::create_dir_all(dir).map_err(|source| {
            ServerError::Snapshot(format!(
                "could not create the snapshot directory {dir:?}: {source}"
            ))
        })?;
        let path = dir.join(snapshot::snapshot_filename(head));
        if path.exists() {
            return Ok(None);
        }
        graph.snapshot_into(&path)?;
        tracing::info!(head = head.0, ?path, "wrote graph snapshot");
        Ok(Some(path))
    }

    /// Run a semantic search over the agent's memory — the engine behind `memory.search`, exposed for
    /// tests and a future operator/console search surface. Embeds the query off every lock, then ranks
    /// under a brief graph + vector-index read lock. Empty on a graph-only instance (no embedder).
    pub async fn search(
        &self,
        query: &str,
        present_set: &[MemoryId],
        limit: usize,
    ) -> Result<Vec<SearchHit>, ServerError> {
        let Some(retrieval) = &self.engine.retrieval else {
            return Ok(Vec::new());
        };
        let embedding = retrieval
            .embedder
            .embed(&[query.to_owned()])
            .await
            .map_err(|error| ServerError::Index(IndexError::Embed(error)))?
            .into_iter()
            .next()
            .unwrap_or_default();
        let settings = Settings::from_store(self.engine.store.lock().as_ref())?.search;
        let now = self.engine.clock.now();
        let request = SearchQuery {
            text: query,
            embedding: &embedding,
            namespace: None,
            tags: &[],
            present_set,
        };
        // Graph before the vector index — the lock order search and the indexer share; held only across
        // the synchronous ranking, never an await.
        let graph = self.engine.graph.lock();
        let vectors = retrieval.vectors.lock();
        Ok(rank_search(
            &graph,
            vectors.as_ref(),
            &request,
            &settings,
            now,
            limit,
        )?)
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
    pub fn platform(&self) -> Platform<'_> {
        Platform { server: self }
    }
}

/// One routed turn's inputs: the `conversation` it lands in, who is `present_set` (for the session
/// brief), the `participant` it is attributed to, the `inbound` text, and the `template`/`authority`
/// that frame it — `Scaffold`/`Platform` for an ordinary message, `Imprint`/`Operator` for the
/// console interview. Bundled so [`Server::run_session_turn`] takes the routed turn as a whole.
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
            .ensure_session(routed.conversation, routed.present_set, model)
            .await?;
        let settings = Settings::from_store(self.engine.store.lock().as_ref())?;
        let turn_settings = settings.turn;
        let max_steps = turn_settings.max_steps as usize;
        let block_timeout = Duration::from_secs(turn_settings.block_timeout_seconds.max(0) as u64);
        let max_block_attempts = turn_settings.max_block_attempts.max(1) as u32;
        let capture = settings.observability.capture_model_calls;
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
            session_started_at: open.started_at,
            buffer: &buffer,
            template: routed.template,
            authority: routed.authority,
            present_set: routed.present_set,
            max_steps,
            block_timeout,
            max_block_attempts,
            capture,
        })
        .await?;
        Ok((report, buffer))
    }

    /// A fresh session VM for a conversation, carrying the MCP projection when servers are connected.
    fn mint_vm(&self, conversation: ConversationId) -> Session {
        match &self.mcp {
            Some(runtime) => Session::with_mcp(
                conversation,
                runtime.host.clone(),
                runtime.catalogue.clone(),
            ),
            None => Session::new(conversation),
        }
    }

    /// Flush a closing session's working state to memory, then record `SessionEnded`. The budget-gated
    /// pre-compaction flush gives a substantive session (at least `flush_min_turns`) one turn to write
    /// durable memory before the cut, so nothing it learned is lost between its last write and the next
    /// conversation; a light session skips it, so the hot-path model call is paid only when there is
    /// state worth saving. The flush runs **before** `SessionEnded`, so a flush failure leaves the
    /// session standing for a retry rather than dropping its state. Shared by the budget-compaction
    /// close (which then stages a carryover) and the idle/recovery closes (which do not). The caller
    /// has already removed `open` from the sessions map. Returns whether the flush ran.
    async fn flush_and_end(
        &self,
        conversation: ConversationId,
        open: &OpenSession,
        model: &dyn ModelClient,
    ) -> Result<bool, ServerError> {
        let settings = Settings::from_store(self.engine.store.lock().as_ref())?;
        let buffer = buffer_turns(
            self.engine.store.lock().as_ref(),
            conversation,
            open.start_seq,
        )?;
        let flushed = buffer.len() as i64 >= settings.compaction.flush_min_turns;
        if flushed {
            let present_set = self.engine.graph.lock().session_participants(open.id)?;
            run_flush(Flush {
                session: &open.vm,
                model,
                engine: self.engine.clone(),
                brief: &open.brief,
                session_started_at: open.started_at,
                buffer: &buffer,
                present_set: &present_set,
                max_steps: settings.turn.max_steps as usize,
                block_timeout: Duration::from_secs(
                    settings.turn.block_timeout_seconds.max(0) as u64
                ),
                max_block_attempts: settings.turn.max_block_attempts.max(1) as u32,
                capture: settings.observability.capture_model_calls,
            })
            .await?;
            self.engine
                .graph
                .lock()
                .materialize_from(self.engine.store.lock().as_ref())?;
        }
        open.vm.shutdown_mcp().await;
        let now = self.engine.clock.now();
        self.engine.store.lock().append(
            now,
            vec![EventPayload::SessionEnded {
                conversation,
                id: open.id,
            }],
        )?;
        Ok(flushed)
    }

    /// Ensure a live session for `conversation`. Reuse the open one if activity is within the idle gap.
    /// Otherwise, on a cold start (no live session in this process), recover a session still open in the
    /// log — left by a restart or a passive graceful exit: within the idle gap resume it untouched (an
    /// identical prompt prefix, so the serving cache survives the restart), past it close-with-flush.
    /// Then, for a stale live session or after a recovered close, open a fresh one — composing and
    /// freezing its brief and minting a fresh VM. Boundaries are recorded (`SessionStarted` /
    /// `SessionEnded`), never recomputed at replay.
    async fn ensure_session(
        &self,
        conversation: ConversationId,
        present_set: &[MemoryId],
        model: &dyn ModelClient,
    ) -> Result<Arc<OpenSession>, ServerError> {
        let now = self.engine.clock.now();
        let settings = Settings::from_store(self.engine.store.lock().as_ref())?;
        let idle_gap_ms = settings.compaction.idle_gap_seconds.saturating_mul(1_000);

        // Reuse the open session if its last activity is within the idle gap, bumping it. The map
        // guard is released before returning; the returned `Arc` keeps the session alive for the turn.
        // A stale live session is noted (`live_present`) so the cold-start recovery below runs only for
        // a true cold start — a stale live one is closed-and-reopened by the path further down.
        let live_present = {
            let sessions = self.sessions.lock();
            match sessions.get(&conversation) {
                Some(open) if now.as_millis() - open.last_activity_millis() <= idle_gap_ms => {
                    open.touch(now);
                    return Ok(open.clone());
                }
                other => other.is_some(),
            }
        };

        // Cold start with a session still open in the log (a restart, or a passive graceful exit that
        // left it open — resolution is deliberately lazy, on this next message). Recover it: within the
        // idle gap resume it untouched so the prompt prefix is byte-identical; past it (or a seeded
        // compaction continuation, not byte-faithfully resumable from its seq alone) close it with a
        // flush so its working state is consolidated before the fresh session opens below.
        // Resolve the recovery target before the body, so the graph guard is dropped before the
        // flush's `.await` below (a guard held across an await would make the turn future non-Send).
        let recovered = if live_present {
            None
        } else {
            self.engine.graph.lock().last_open_session(conversation)?
        };
        if let Some(recovered) = recovered {
            let buffer = buffer_turns(
                self.engine.store.lock().as_ref(),
                conversation,
                recovered.start_seq,
            )?;
            let last_activity = buffer
                .last()
                .map_or(recovered.started_at, |turn| turn.recorded_at);
            let resumable =
                !recovered.seeded && now.as_millis() - last_activity.as_millis() <= idle_gap_ms;
            let open = OpenSession {
                id: recovered.id,
                vm: self.mint_vm(conversation),
                brief: recovered.brief,
                started_at: recovered.started_at,
                last_activity: AtomicI64::new(last_activity.as_millis()),
                start_seq: recovered.start_seq,
            };
            if resumable {
                open.touch(now);
                let open = Arc::new(open);
                self.sessions.lock().insert(conversation, open.clone());
                tracing::info!(?conversation, session = ?open.id, "resumed an open session after a cold start");
                return Ok(open);
            }
            self.flush_and_end(conversation, &open, model).await?;
            tracing::info!(?conversation, session = ?open.id, "flushed and closed a stale recovered session");
        }

        // Catch the wake-up scheduler up to now before the session opens, so a just-due item can
        // surface in this session if it is eligible (the drain below reads the fired surface). The
        // background driver ([`Server::run_scheduler`]) fires continuously on a timer; this catch-up
        // stays for immediacy at session open and is idempotent with it.
        self.fire_due_now(now)?;

        // A lapsed live session ends before the new one opens: take it out under the map guard (so no
        // guard is held across the flush's `.await`), then flush-and-end it — the idle close now
        // consolidates its working state too, not only the budget-compaction close.
        let old = self.sessions.lock().remove(&conversation);
        if let Some(old) = old {
            self.flush_and_end(conversation, old.as_ref(), model)
                .await?;
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

        // Force the description catch-up to completion before composing the brief, so it never reads
        // stale prose for memories a prior turn or the pre-compaction flush just wrote (spec
        // §Starvation bound → composing a brief forces the catch-up). Then materialize the fresh
        // descriptions into the graph the brief reads. (A full catch-up here; narrowing it to the
        // brief's own memories is a later refinement.) No lock is held across the model call.
        self.describe_catch_up(model).await?;
        self.engine
            .graph
            .lock()
            .materialize_from(self.engine.store.lock().as_ref())?;

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
        let vm = self.mint_vm(conversation);
        let open = Arc::new(OpenSession {
            id,
            vm,
            brief,
            started_at: now,
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

    /// Resolve the console operator's stable `person/operator` stub, minting it once on the
    /// first imprint. Unlike a platform participant it carries no `ParticipantIdentified` binding —
    /// the operator has no platform identity, must never collide with a real participant, and must
    /// resolve identically across imprints — so it is keyed only by its canonical name.
    fn resolve_or_mint_operator(&self) -> Result<MemoryId, ServerError> {
        let operator = Namespace::Person.handle("operator");
        if let Some(memory) = self.engine.graph.lock().memory_by_name(operator.as_str())? {
            return Ok(memory.id);
        }
        let id = MemoryId::generate();
        let now = self.engine.clock.now();
        self.engine.store.lock().append(
            now,
            vec![EventPayload::MemoryCreated { id, name: operator }],
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

    /// Take a snapshot if enough events have accrued since the last one — the activity gate that keeps
    /// idle periods from snapshotting (spec §Snapshots). Compares the graph head to the newest existing
    /// snapshot's head; when the gap meets `min_new_events`, writes a snapshot and prunes to `keep`.
    /// Returns whether one was written.
    fn snapshot_if_due(&self, schedule: &SnapshotSchedule) -> Result<bool, ServerError> {
        let head = self.engine.graph.lock().head()?;
        let last = snapshot::latest(&schedule.dir)
            .map_err(|error| ServerError::Snapshot(error.to_string()))?
            .map_or(0, |(_, head)| head.0);
        if head.0.saturating_sub(last) < schedule.min_new_events {
            return Ok(false);
        }
        let wrote = self.snapshot(&schedule.dir)?.is_some();
        if wrote {
            snapshot::prune(&schedule.dir, schedule.keep)
                .map_err(|error| ServerError::Snapshot(error.to_string()))?;
        }
        Ok(wrote)
    }

    /// The background snapshotter: on each `check_interval` tick, snapshot the graph if activity has
    /// accrued ([`Server::snapshot_if_due`]), stopping on the same shutdown signal as the scheduler.
    /// A failure is logged, not fatal — the log is always the source of truth, so a missed snapshot
    /// only slows the next cold boot.
    pub async fn run_snapshotter(
        self: Arc<Self>,
        schedule: SnapshotSchedule,
        shutdown: impl Future<Output = ()>,
    ) {
        let mut interval = tokio::time::interval(schedule.check_interval);
        tokio::pin!(shutdown);
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    if let Err(error) = self.snapshot_if_due(&schedule) {
                        tracing::error!(%error, "snapshotter: writing a snapshot failed");
                    }
                }
                _ = &mut shutdown => break,
            }
        }
        tracing::info!("snapshotter stopped");
    }

    /// Catch the vector index up to the log (spec §Storage → vector store). Reads the cursor and the
    /// events past it under brief sync locks, **embeds off every lock**, then applies the embedded
    /// batch under a brief vector-index lock. So the slow `embed().await` holds no guard at all — not
    /// the store, not the graph, not the index — and a concurrent `memory.search` never waits behind a
    /// batch's embedding. A no-op returning `0` on a graph-only instance (no embedder configured).
    pub async fn index_catch_up(&self) -> Result<usize, ServerError> {
        let Some(retrieval) = &self.engine.retrieval else {
            return Ok(0);
        };
        let from = retrieval
            .vectors
            .lock()
            .cursor()
            .map_err(IndexError::Vector)?
            .next();
        let events = self
            .engine
            .store
            .lock()
            .read_from(from)
            .map_err(IndexError::Store)?;
        let count = events.len();
        let batch = embed_batch(retrieval.embedder.as_ref(), &events).await?;
        apply_batch(retrieval.vectors.lock().as_mut(), batch).map_err(IndexError::Vector)?;
        Ok(count)
    }

    /// Reconcile the vector index with the configured embedding model, blocking until done. If the
    /// model that produced the stored vectors differs from the configured one, the index lives in a
    /// now-incompatible embedding space — cosine across the two is silently wrong — so this logs an
    /// `EmbeddingModelChanged` migration, clears the index, and re-embeds the whole log under the new
    /// model before returning. Called at boot *before* the server serves, so requests are refused (the
    /// server is not yet up) rather than answered from a mixed or stale space. A no-op when retrieval is
    /// off, the index is empty (nothing to migrate — the indexer will embed fresh), or the model is
    /// unchanged. Returns whether a re-embed ran.
    ///
    /// The simple, downtime-accepting form: the costlier zero-downtime discipline (build the new index
    /// alongside the old, serve the old until an atomic cutover) is a deferred follow-up (spec §Storage
    /// → vector store).
    pub async fn reembed_if_embedding_model_changed(&self) -> Result<bool, ServerError> {
        let Some(retrieval) = &self.engine.retrieval else {
            return Ok(false);
        };
        let configured = retrieval.embedder.model_id();
        let recorded = retrieval
            .vectors
            .lock()
            .model_id()
            .map_err(IndexError::Vector)?;
        match recorded {
            Some(recorded) if recorded.as_str() != configured => {
                tracing::warn!(
                    from = %recorded,
                    to = configured,
                    "embedding model changed; clearing the vector index and re-embedding the log"
                );
                let now = self.engine.clock.now();
                self.engine.store.lock().append(
                    now,
                    vec![EventPayload::EmbeddingModelChanged {
                        from: recorded,
                        to: configured.into(),
                    }],
                )?;
                // Apply the migration into the graph (a no-op there) so graph-head keeps pace with the
                // log, then clear the index and re-embed the whole log under the new model.
                self.engine
                    .graph
                    .lock()
                    .materialize_from(self.engine.store.lock().as_ref())?;
                retrieval
                    .vectors
                    .lock()
                    .clear()
                    .map_err(IndexError::Vector)?;
                let indexed = self.index_catch_up().await?;
                tracing::info!(indexed, "re-embed complete; serving resumes");
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    /// The background indexer: on each tick, catch the vector index up to the log (spec §Storage →
    /// vector store — indexing runs off the turn's hot path). Idempotent and cursor-resumed, so an idle
    /// tick is cheap and the first tick rebuilds a fresh index. Stops on the shutdown signal; a failure
    /// is logged, not fatal — search degrades to slightly stale until the next tick. Returns
    /// immediately on a graph-only instance.
    pub async fn run_indexer(
        self: Arc<Self>,
        interval: Duration,
        shutdown: impl Future<Output = ()>,
    ) {
        if self.engine.retrieval.is_none() {
            return;
        }
        let mut ticker = tokio::time::interval(interval);
        tokio::pin!(shutdown);
        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    match self.index_catch_up().await {
                        Ok(indexed) if indexed > 0 => {
                            tracing::debug!(indexed, "indexer caught the vector index up")
                        }
                        Ok(_) => {}
                        Err(error) => tracing::error!(%error, "indexer: catch-up failed"),
                    }
                }
                _ = &mut shutdown => break,
            }
        }
        tracing::info!("indexer stopped");
    }

    /// Catch synthesized descriptions up to the log: regenerate every memory whose content changed
    /// since the describer's cursor (description, belief arbitration, and temporal extraction), then
    /// advance it (spec §Write path → regenerate off the hot path, as a catch-up). The synchronous
    /// counterpart to the background describer — the same dual-mode shape as `index_catch_up` — driven
    /// explicitly by tests and the eval harness so a caller can force regeneration to a known point and
    /// then read fresh descriptions. Returns how many memories it considered.
    pub async fn describe_catch_up(&self, model: &dyn ModelClient) -> Result<usize, ServerError> {
        let cursor = *self.describer_cursor.lock();
        let (advanced, count) = run_describe_catch_up(&self.engine, model, cursor).await?;
        *self.describer_cursor.lock() = advanced;
        Ok(count)
    }

    /// Seed the describer's cursor to log-head, treating everything written so far as described. Called
    /// at boot and at agent creation so the genesis-seeded `self` (which has no description yet) is not
    /// regenerated by a synchronous catch-up before any real content is written.
    pub(crate) fn baseline_describer_cursor(&self) -> Result<(), ServerError> {
        *self.describer_cursor.lock() = self.engine.store.lock().head()?;
        Ok(())
    }

    /// The background describer: on each tick, catch synthesized descriptions up to the log off the
    /// turn's hot path (spec §Write path). Idempotent and cursor-resumed, so an idle tick is cheap.
    /// Stops on the shutdown signal; a failure is logged, not fatal — a description stays stale until
    /// the next tick or the forcing guard before a brief.
    pub async fn run_describer(
        self: Arc<Self>,
        model: Arc<dyn ModelClient>,
        interval: Duration,
        shutdown: impl Future<Output = ()>,
    ) {
        let mut ticker = tokio::time::interval(interval);
        tokio::pin!(shutdown);
        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    match self.describe_catch_up(model.as_ref()).await {
                        Ok(regenerated) if regenerated > 0 => {
                            tracing::debug!(regenerated, "describer caught descriptions up")
                        }
                        Ok(_) => {}
                        Err(error) => tracing::error!(%error, "describer: catch-up failed"),
                    }
                }
                _ = &mut shutdown => break,
            }
        }
        tracing::info!("describer stopped");
    }

    /// Catch merge adjudications up to the log off the hot path (spec §Cross-platform identity →
    /// adjudicated merge): weigh every proposed merge written since the cursor, advancing it. Driven on
    /// a timer by the served runtime and explicitly by tests and the eval harness. Returns how many
    /// proposals it considered.
    pub async fn adjudicate_catch_up(&self, model: &dyn ModelClient) -> Result<usize, ServerError> {
        let cursor = *self.adjudicator_cursor.lock();
        let (advanced, count) = run_adjudicate_catch_up(&self.engine, model, cursor).await?;
        *self.adjudicator_cursor.lock() = advanced;
        Ok(count)
    }

    /// Seed the adjudicator's cursor to log-head, treating every proposal so far as already adjudicated.
    /// Called at boot and at agent creation, like the describer's, so a restart does not re-weigh old
    /// proposals.
    pub(crate) fn baseline_adjudicator_cursor(&self) -> Result<(), ServerError> {
        *self.adjudicator_cursor.lock() = self.engine.store.lock().head()?;
        Ok(())
    }

    /// The background adjudicator: on each tick, weigh proposed merges off the hot path. Idempotent and
    /// cursor-resumed, so an idle tick is cheap; a failure is logged, not fatal — a proposal stays
    /// pending until the next tick or an operator decides.
    pub async fn run_adjudicator(
        self: Arc<Self>,
        model: Arc<dyn ModelClient>,
        interval: Duration,
        shutdown: impl Future<Output = ()>,
    ) {
        let mut ticker = tokio::time::interval(interval);
        tokio::pin!(shutdown);
        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    match self.adjudicate_catch_up(model.as_ref()).await {
                        Ok(considered) if considered > 0 => {
                            tracing::debug!(considered, "adjudicator weighed merge proposals")
                        }
                        Ok(_) => {}
                        Err(error) => tracing::error!(%error, "adjudicator: catch-up failed"),
                    }
                }
                _ = &mut shutdown => break,
            }
        }
        tracing::info!("adjudicator stopped");
    }

    /// Tear down the live sessions at server shutdown: drain the session map and shut each session's
    /// MCP instances down (best-effort). Called by the serving host once the HTTP server has stopped
    /// accepting. Dropping the drained sessions also releases their VMs.
    pub async fn shutdown(&self) {
        let sessions: Vec<Arc<OpenSession>> = self
            .sessions
            .lock()
            .drain()
            .map(|(_, session)| session)
            .collect();
        for session in &sessions {
            session.vm.shutdown_mcp().await;
        }
        drop(sessions);
    }
}

/// The initial stream-limit permit count read from settings at construction. Floors at 1 so a
/// missing, zero, or negative configuration never produces a deadlocking zero-permit semaphore; a
/// store read failure falls back to the build default with a warning.
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
struct OpenSession {
    id: SessionId,
    vm: Session,
    brief: String,
    /// When the session opened — the time frozen into the system prompt's "the session begins on …",
    /// so every turn in the session sends an identical system prefix (the live wall clock rides in the
    /// per-message stamps instead). Holding it stable is what lets the serving layer reuse the prefix
    /// cache across the session's turns.
    started_at: Timestamp,
    /// The last-activity wall-clock in epoch millis, the idle-gap is measured from. Atomic so the
    /// idle-reuse path can bump it through the shared `&OpenSession` without a map-wide write lock.
    last_activity: AtomicI64,
    /// The log seq the live buffer is read from: the `SessionStarted` seq for a fresh or idle-opened
    /// session, or a carried tail's seq across a compaction seam (so the carryover plus this
    /// session's turns reconstruct the buffer — see [`buffer_turns`]).
    start_seq: Seq,
}

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
    Turn(TurnError),
    /// Connecting the MCP servers failed (a probe-level hard error, e.g. a stale `allow`/`deny`).
    Mcp(crate::mcp::McpError),
    /// Writing a graph snapshot failed (creating the directory, or the `VACUUM INTO` itself).
    Snapshot(String),
    /// Catching the vector index up to the log failed (embedding, the vector store, or the log read).
    Index(IndexError),
    /// A semantic search failed (the graph projection or the vector index).
    Search(SearchError),
    /// An operator Lua console block failed at the VM level (a script error reaches the operator as
    /// a result, not this; this is an infrastructure failure running the block).
    Lua(crate::agent::lua::LuaError),
}

impl std::fmt::Display for ServerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ServerError::Store(error) => write!(f, "server (store): {error}"),
            ServerError::Graph(error) => write!(f, "server (graph): {error}"),
            ServerError::Turn(error) => write!(f, "server (turn): {error}"),
            ServerError::Mcp(error) => write!(f, "server (mcp): {error}"),
            ServerError::Snapshot(message) => write!(f, "server (snapshot): {message}"),
            ServerError::Index(error) => write!(f, "server (index): {error}"),
            ServerError::Search(error) => write!(f, "server (search): {error}"),
            ServerError::Lua(error) => write!(f, "server (lua): {error}"),
        }
    }
}

impl std::error::Error for ServerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ServerError::Store(error) => Some(error),
            ServerError::Graph(error) => Some(error),
            ServerError::Turn(error) => Some(error),
            ServerError::Mcp(error) => Some(error),
            ServerError::Snapshot(_) => None,
            ServerError::Index(error) => Some(error),
            ServerError::Search(error) => Some(error),
            ServerError::Lua(error) => Some(error),
        }
    }
}

impl From<SearchError> for ServerError {
    fn from(error: SearchError) -> Self {
        ServerError::Search(error)
    }
}

impl From<IndexError> for ServerError {
    fn from(error: IndexError) -> Self {
        ServerError::Index(error)
    }
}

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
impl From<IdentityError> for ServerError {
    fn from(error: IdentityError) -> Self {
        match error {
            IdentityError::Store(error) => ServerError::Store(error),
            IdentityError::Graph(error) => ServerError::Graph(error),
        }
    }
}

impl From<BriefError> for ServerError {
    fn from(error: BriefError) -> Self {
        match error {
            BriefError::Graph(error) => ServerError::Graph(error),
        }
    }
}

impl From<SchedulerError> for ServerError {
    fn from(error: SchedulerError) -> Self {
        match error {
            SchedulerError::Store(error) => ServerError::Store(error),
            SchedulerError::Graph(error) => ServerError::Graph(error),
        }
    }
}

impl From<TurnError> for ServerError {
    fn from(error: TurnError) -> Self {
        ServerError::Turn(error)
    }
}

impl From<crate::agent::lua::LuaError> for ServerError {
    fn from(error: crate::agent::lua::LuaError) -> Self {
        ServerError::Lua(error)
    }
}

#[cfg(test)]
mod embedding_swap_tests {
    use std::sync::Arc;

    use async_trait::async_trait;

    use super::*;
    use crate::{
        clock::ManualClock,
        graph::Graph,
        model::{ModelError, embed::Embedding},
        vector::{InMemoryVectorIndex, VectorId, VectorRecord},
    };

    /// An embedder whose `model_id` is configurable, so a test can stand for a model swap; its vectors
    /// are constant and never actually compared, only counted and tagged.
    struct TaggedEmbedder {
        id: &'static str,
        dims: usize,
    }

    #[async_trait]
    impl Embedder for TaggedEmbedder {
        fn dimensions(&self) -> usize {
            self.dims
        }

        fn model_id(&self) -> &str {
            self.id
        }

        async fn embed(&self, inputs: &[String]) -> Result<Vec<Embedding>, ModelError> {
            Ok(inputs.iter().map(|_| vec![0.1; self.dims]).collect())
        }
    }

    fn server_over(
        store: MemoryStore,
        vectors: InMemoryVectorIndex,
        model: &'static str,
        dims: usize,
    ) -> Server {
        Server::with_retrieval(
            Box::new(store),
            Graph::open_in_memory().unwrap(),
            Box::new(ManualClock::new(Timestamp::from_millis(2_000))),
            Arc::new(TaggedEmbedder { id: model, dims }),
            Box::new(vectors),
        )
    }

    #[tokio::test]
    async fn a_swap_logs_the_change_and_reembeds_under_the_new_model() {
        let dims = 8;
        // A log with one embeddable description.
        let mut store = MemoryStore::new();
        let mem = MemoryId::generate();
        store
            .append(
                Timestamp::from_millis(1_000),
                vec![EventPayload::MemoryDescriptionRegenerated {
                    id: mem,
                    new_text: "an avid climber".to_owned(),
                    produced_by: None,
                }],
            )
            .unwrap();
        // An index that a prior model already built over that log.
        let mut vectors = InMemoryVectorIndex::new();
        vectors
            .upsert(VectorRecord {
                id: VectorId::new("desc:stale"),
                embedding: vec![0.5; dims],
                model_id: "old-model".into(),
            })
            .unwrap();
        vectors.set_cursor(store.head().unwrap()).unwrap();

        let server = server_over(store, vectors, "new-model", dims);
        let reembedded = server.reembed_if_embedding_model_changed().await.unwrap();
        assert!(reembedded, "a model change must trigger a re-embed");

        // The swap is logged, old → new.
        let events = server.engine.store.lock().read_from(Seq::ZERO).unwrap();
        let logged = events.iter().find_map(|event| match &event.payload {
            EventPayload::EmbeddingModelChanged { from, to } => {
                Some((from.to_string(), to.to_string()))
            }
            _ => None,
        });
        assert_eq!(
            logged,
            Some(("old-model".to_owned(), "new-model".to_owned()))
        );

        // The index was cleared of the stale vector and rebuilt under the new model.
        let vectors = server.engine.retrieval.as_ref().unwrap();
        assert_eq!(vectors.vectors.lock().len().unwrap(), 1);
        assert_eq!(
            vectors.vectors.lock().model_id().unwrap().as_deref(),
            Some("new-model")
        );

        // A second boot finds the model unchanged and does nothing.
        assert!(!server.reembed_if_embedding_model_changed().await.unwrap());
    }

    #[tokio::test]
    async fn an_unchanged_model_is_a_noop_and_an_empty_index_needs_no_migration() {
        let dims = 8;
        // Unchanged model over a populated index: no-op.
        let mut vectors = InMemoryVectorIndex::new();
        vectors
            .upsert(VectorRecord {
                id: VectorId::new("desc:x"),
                embedding: vec![0.5; dims],
                model_id: "same-model".into(),
            })
            .unwrap();
        let server = server_over(MemoryStore::new(), vectors, "same-model", dims);
        assert!(!server.reembed_if_embedding_model_changed().await.unwrap());

        // Empty index (a fresh agent): nothing to migrate, even under a "different" model.
        let fresh = server_over(
            MemoryStore::new(),
            InMemoryVectorIndex::new(),
            "any-model",
            dims,
        );
        assert!(!fresh.reembed_if_embedding_model_changed().await.unwrap());
    }

    /// The end-to-end path on the real backends across a restart: a log embedded under one model on
    /// disk, reopened under another, is detected and re-embedded — exercising the persisted sqlite
    /// store, graph, and vec0 index, not just the in-memory fakes.
    #[tokio::test]
    async fn a_swap_is_detected_and_rebuilt_across_a_real_sqlite_restart() {
        use crate::{store::SqliteStore, vector::SqliteVectorIndex};

        let dims = 8;
        let tag = MemoryId::generate().0;
        let dir = std::env::temp_dir();
        let log = dir.join(format!("zuihitsu-emc-log-{tag}.sqlite"));
        let graph_path = dir.join(format!("zuihitsu-emc-graph-{tag}.sqlite"));
        let vecs = dir.join(format!("zuihitsu-emc-vecs-{tag}.sqlite"));

        // Phase 1 — build a log with one embeddable description and index it under model "old", all on
        // disk; then drop the server so the file locks release.
        {
            let mut store = SqliteStore::open(&log).unwrap();
            let mem = MemoryId::generate();
            store
                .append(
                    Timestamp::from_millis(1_000),
                    vec![
                        EventPayload::MemoryCreated {
                            id: mem,
                            name: Namespace::Topic.handle("x"),
                        },
                        EventPayload::MemoryDescriptionRegenerated {
                            id: mem,
                            new_text: "an avid climber".to_owned(),
                            produced_by: None,
                        },
                    ],
                )
                .unwrap();
            let server = Server::with_retrieval(
                Box::new(store),
                Graph::open(&graph_path).unwrap(),
                Box::new(ManualClock::new(Timestamp::from_millis(1_000))),
                Arc::new(TaggedEmbedder { id: "old", dims }),
                Box::new(SqliteVectorIndex::open(&vecs, dims).unwrap()),
            );
            server.index_catch_up().await.unwrap();
            let retrieval = server.engine.retrieval.as_ref().unwrap();
            assert_eq!(
                retrieval.vectors.lock().model_id().unwrap().as_deref(),
                Some("old"),
                "phase 1 should embed under the old model"
            );
        }

        // Phase 2 — restart over the same files under model "new": boot, then the blocking re-embed.
        {
            let vectors = SqliteVectorIndex::open(&vecs, dims).unwrap();
            assert_eq!(
                vectors.model_id().unwrap().as_deref(),
                Some("old"),
                "the persisted index should carry the old model across the restart"
            );
            let mut server = Server::with_retrieval(
                Box::new(SqliteStore::open(&log).unwrap()),
                Graph::open(&graph_path).unwrap(),
                Box::new(ManualClock::new(Timestamp::from_millis(2_000))),
                Arc::new(TaggedEmbedder { id: "new", dims }),
                Box::new(vectors),
            );
            server.boot().unwrap();
            assert!(server.reembed_if_embedding_model_changed().await.unwrap());

            let events = server.engine.store.lock().read_from(Seq::ZERO).unwrap();
            assert!(
                events.iter().any(|event| matches!(
                    &event.payload,
                    EventPayload::EmbeddingModelChanged { from, to }
                        if from.as_str() == "old" && to.as_str() == "new"
                )),
                "the swap should be logged old → new"
            );
            let retrieval = server.engine.retrieval.as_ref().unwrap();
            assert_eq!(
                retrieval.vectors.lock().model_id().unwrap().as_deref(),
                Some("new"),
                "the index should be rebuilt under the new model"
            );
            assert_eq!(retrieval.vectors.lock().len().unwrap(), 1);
        }

        for path in [&log, &graph_path, &vecs] {
            let _ = std::fs::remove_file(path);
            let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
            let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
        }
    }
}
