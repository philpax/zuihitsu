//! The agent instance: the single writer that owns the event log, the materialized graph, and the
//! clock, and exposes its API split by client authority (spec §Clients and the instance boundary).
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
use tracing::Instrument;

use crate::{
    InstanceFeatures,
    agent::{
        Flush, McpCatalogue, Turn, TurnError, TurnOutcome, TurnReport, TurnView, buffer_turns,
        genesis::{self, GenesisStatus},
        lua::Session,
        run_adjudicate_catch_up, run_describe_catch_up, run_flush, run_link_inference_catch_up,
        run_turn,
    },
    clock::Clock,
    engine::Engine,
    event::{EventPayload, Initiation, PromptTemplateName, TurnRole},
    graph::{Graph, GraphError},
    ids::{ConversationId, MemoryId, MemoryName, NamespacedMemoryName, Seq, SessionId, TurnId},
    mcp::{McpHost, McpServerConfig},
    memory::{
        brief::{self, BriefError},
        identity::IdentityError,
        memory_block::Authority,
        scheduler::{self, SchedulerError},
        search::{SearchError, SearchHit, SearchQuery, search as rank_search},
    },
    metrics::{
        observe_flush_turn, observe_search, observe_session_closed, observe_session_opened,
        observe_turn, observe_turn_error, observe_wakeups_fired, observe_wakeups_surfaced,
        observe_worker_error,
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

pub struct Instance {
    // The store, graph, and clock bundled behind one shared [`Engine`], so a turn shares them with a
    // single pointer bump and the Lua block API can hold a `'static` handle across `eval_async`. The
    // instance is still the single writer; the engine's mutexes serialize access rather than admit a
    // second writer. See [`Engine`] for the graph-before-store lock-ordering rule.
    engine: Arc<Engine>,
    /// The live session per conversation: its id, the VM whose globals persist across the session's
    /// turns, the frozen brief, and the last-activity time the idle-gap is measured from. Pure
    /// runtime state — never logged (the `SessionStarted` / `SessionEnded` events are); an agent
    /// restart drops the map, but the next message recovers a session still open in the log through
    /// `ensure_session` (resumed within the idle gap, else closed-with-flush and reopened). Behind a
    /// `Mutex` (and each value an `Arc`) so concurrent conversations reach the map through a shared
    /// `&Instance`; a turn holds its session's `Arc` across the turn `.await` without keeping the map guard.
    sessions: Mutex<HashMap<ConversationId, Arc<OpenSession>>>,
    /// A per-conversation async lock serializing its session lifecycle: the close-with-flush of one
    /// session and the open of the next. A close runs a flush — a model call lasting seconds — before it
    /// records `SessionEnded`, and within that window the idle sweep and the message-driven recovery path
    /// both reach the close for the same session. Held across `ensure_session` and the sweep's close, it
    /// makes the message path *wait* for an in-flight sweep close to finish before opening the next
    /// session — so that session's brief reads the flush's writes — and lets the second closer see the
    /// session already ended and skip. Locks are minted lazily and kept (one per conversation the agent
    /// ever holds; negligible).
    lifecycle: Mutex<HashMap<ConversationId, Arc<tokio::sync::Mutex<()>>>>,
    /// Carryover staged by a token-triggered compaction, consumed by the next `ensure_session` to
    /// seed the re-segmented session (spec §Compaction). Keyed by conversation; an entry lives only
    /// between the compacting turn and the next message in that room. Behind a `Mutex` for the same
    /// shared-`&Instance` reason as `sessions`.
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
    /// The link-inference pass's cursor: the log seq through which implicit relationships have been
    /// inferred and linked. Its own background pass (and the explicit `link_inference_catch_up`)
    /// advances it as it extracts relationships off the hot path (spec §Write path → link inference).
    /// Re-seeded to log-head at boot, like the describer's and adjudicator's.
    link_inference_cursor: Mutex<Seq>,
    /// Serializes describe-catch-up passes — the force-before-brief one (a new session opening) and the
    /// background timer one — so two never run concurrently. Without it both read the cursor before
    /// either advances it, so they re-describe (and re-embed) the same memories for the same change.
    describe_guard: tokio::sync::Mutex<()>,
    /// As `describe_guard`, serializing the adjudicator's catch-up.
    adjudicate_guard: tokio::sync::Mutex<()>,
    /// As `describe_guard`, serializing the link-inference catch-up.
    link_inference_guard: tokio::sync::Mutex<()>,
    /// The concurrent-stream limit (spec §Concurrency): a permit is held for each in-flight inbound
    /// message's whole handling, so no more than `max_concurrent_streams` turns crowd the shared
    /// model at once; further streams queue. Sized from settings at construction (a change takes
    /// effect on restart).
    streams: Semaphore,
    /// The MCP host and the catalogue probed from it at [`Instance::connect_mcp`] — `None` until then.
    /// Each session opened while it is set gets the `mcp.<server>.*` projection over the same catalogue.
    mcp: Option<McpRuntime>,
    /// Which API features this instance enables — gates the Lua functions installed per block, the
    /// API reference rendered into the system prompt, and (at genesis) the scaffold dotpoints. Set
    /// at construction, before genesis, so the baked scaffold reflects it; defaults to all-on.
    features: InstanceFeatures,
    /// The configured model's context window, in tokens, set by the serving host from `[model]`
    /// config. `None` for an in-memory or model-less instance. Genesis derives the agent's initial
    /// compaction budget from it (a fraction of the window); see [`Control::create_agent`].
    model_context_length: Option<u32>,
}

/// The connected MCP runtime: the host that spawns server instances and the catalogue probed from it
/// once at startup (shared into every session opened thereafter).
struct McpRuntime {
    host: Arc<dyn McpHost>,
    catalogue: McpCatalogue,
}

/// The background snapshotter's policy (spec §Snapshots): where to write, how often to check, the
/// activity gate, and retention. Assembled by the serving host from the `[snapshots]` config and
/// handed to [`Instance::run_snapshotter`].
pub struct SnapshotSchedule {
    pub dir: PathBuf,
    pub check_interval: Duration,
    pub min_new_events: u64,
    pub keep: usize,
}

impl Instance {
    pub fn new(store: Box<dyn Store>, graph: Graph, clock: Box<dyn Clock>) -> Instance {
        Instance::with_features(store, graph, clock, InstanceFeatures::default())
    }

    /// As [`Instance::new`], but with an explicit feature set — the gate that controls which Lua API
    /// features the agent sees. Set before genesis so the baked scaffold reflects it.
    pub fn with_features(
        store: Box<dyn Store>,
        graph: Graph,
        clock: Box<dyn Clock>,
        features: InstanceFeatures,
    ) -> Instance {
        Instance::from_engine(Engine::new(store, graph, clock), features)
    }

    /// As [`Instance::new`], with the semantic-retrieval backends attached — the live instance's
    /// configuration when an embedding endpoint is set, so `memory.search` and the background indexer
    /// have an embedder and a vector index to work over.
    pub fn with_retrieval(
        store: Box<dyn Store>,
        graph: Graph,
        clock: Box<dyn Clock>,
        embedder: Arc<dyn Embedder>,
        vectors: Box<dyn VectorIndex>,
    ) -> Instance {
        Instance::with_retrieval_features(
            store,
            graph,
            clock,
            embedder,
            vectors,
            InstanceFeatures::default(),
        )
    }

    /// As [`Instance::with_retrieval`], but with an explicit feature set.
    pub fn with_retrieval_features(
        store: Box<dyn Store>,
        graph: Graph,
        clock: Box<dyn Clock>,
        embedder: Arc<dyn Embedder>,
        vectors: Box<dyn VectorIndex>,
        features: InstanceFeatures,
    ) -> Instance {
        Instance::from_engine(
            Engine::with_retrieval(store, graph, clock, embedder, vectors),
            features,
        )
    }

    fn from_engine(engine: Arc<Engine>, features: InstanceFeatures) -> Instance {
        let streams = Semaphore::new(initial_stream_permits(&engine));
        Instance {
            engine,
            sessions: Mutex::new(HashMap::new()),
            lifecycle: Mutex::new(HashMap::new()),
            pending_carryover: Mutex::new(HashMap::new()),
            describer_cursor: Mutex::new(Seq::ZERO),
            adjudicator_cursor: Mutex::new(Seq::ZERO),
            link_inference_cursor: Mutex::new(Seq::ZERO),
            describe_guard: tokio::sync::Mutex::new(()),
            adjudicate_guard: tokio::sync::Mutex::new(()),
            link_inference_guard: tokio::sync::Mutex::new(()),
            streams,
            mcp: None,
            features,
            model_context_length: None,
        }
    }

    /// Set the configured model's context window (tokens), from which genesis derives a new agent's
    /// compaction budget. The serving host calls this from `[model]` config before serving; an
    /// in-memory or model-less instance leaves it unset.
    pub fn set_model_context_length(&mut self, context_length: u32) {
        self.model_context_length = Some(context_length);
    }

    /// Connect the configured MCP servers: probe each one's tool catalogue once through `host` (spec
    /// §startup probe), then project that catalogue into every session opened from now on. Called once
    /// after construction by whoever drives serving. A probe-level hard error (a stale `allow`/`deny`,
    /// a duplicate escaped tool name) is surfaced; a server that simply fails to spawn is dropped.
    pub async fn connect_mcp(
        &mut self,
        host: Arc<dyn McpHost>,
        configs: BTreeMap<String, McpServerConfig>,
    ) -> Result<(), InstanceError> {
        let catalogue = McpCatalogue::probe(host.as_ref(), &configs).await?;
        self.mcp = Some(McpRuntime { host, catalogue });
        Ok(())
    }

    /// An instance backed entirely in memory (in-memory store and graph), for tests.
    pub fn in_memory(clock: Box<dyn Clock>) -> Result<Instance, InstanceError> {
        Ok(Instance::new(
            Box::new(MemoryStore::new()),
            Graph::open_in_memory()?,
            clock,
        ))
    }

    /// Catch the graph up to log-head — reconciling a graph left stale or half-applied by a crash
    /// in the commit window — and classify the log for the caller to act on. The single-writer log
    /// lock is acquired when the (file-backed) store is opened, before the instance is constructed.
    pub fn boot(&mut self) -> Result<GenesisStatus, InstanceError> {
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
        self.baseline_link_inference_cursor()?;
        let status = genesis::status(self.engine.store.lock().as_ref())?;
        tracing::info!(?status, applied, "instance booted");
        Ok(status)
    }

    /// Write a graph snapshot into `dir` and return its path, or `None` when the graph is already
    /// snapshotted at its current head (no events since the last one — nothing to checkpoint). Holds
    /// the graph lock across the `VACUUM INTO`, so the capture is at a clean `seq` boundary: a commit,
    /// which takes the same lock, can neither be in flight nor interleave (spec §Snapshots). Creates
    /// `dir` if absent.
    pub fn snapshot(&self, dir: &Path) -> Result<Option<PathBuf>, InstanceError> {
        let graph = self.engine.graph.lock();
        let head = graph.head()?;
        std::fs::create_dir_all(dir).map_err(|source| {
            InstanceError::Snapshot(format!(
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
    ) -> Result<Vec<SearchHit>, InstanceError> {
        let Some(retrieval) = &self.engine.retrieval else {
            return Ok(Vec::new());
        };
        let started = std::time::Instant::now();
        let embedding = retrieval
            .embedder
            .embed(&[query.to_owned()])
            .await
            .map_err(|error| InstanceError::Index(IndexError::Embed(error)))?
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
        let hits = rank_search(&graph, vectors.as_ref(), &request, &settings, now, limit)?;
        observe_search(started.elapsed());
        Ok(hits)
    }

    /// The operator-authority API facet. Takes `&self` so a shared `Arc<Instance>` can hand out a facet
    /// per caller; the server's mutable runtime state lives behind its own locks.
    pub fn control(&self) -> Control<'_> {
        Control { server: self }
    }

    /// Each connected MCP server's name and projected tool count, for the boot log. Empty when no
    /// servers are configured.
    pub fn mcp_summary(&self) -> Vec<(String, usize)> {
        self.mcp
            .as_ref()
            .map(|runtime| runtime.catalogue.server_tool_counts())
            .unwrap_or_default()
    }

    /// The platform-authority API facet — delivering participant turns. It structurally lacks
    /// Control's creation and inspection methods, which is what makes "the operator has no platform
    /// identity" enforceable. Takes `&self` so concurrent conversations each obtain one from a shared
    /// `Arc<Instance>`.
    pub fn platform(&self) -> Platform<'_> {
        Platform { server: self }
    }
}

/// One routed turn's inputs: the `conversation` it lands in, who is `present_set` (for the session
/// brief), the `participant` it is attributed to, the `inbound` text, and the `template`/`authority`
/// that frame it — `Scaffold`/`Platform` for an ordinary message, `Imprint`/`Operator` for the
/// console interview. Bundled so [`Instance::run_session_turn`] takes the routed turn as a whole.
struct RoutedTurn<'a> {
    conversation: ConversationId,
    present_set: &'a [MemoryId],
    participant: MemoryId,
    inbound: &'a str,
    template: PromptTemplateName,
    authority: Authority,
}

/// The session machinery shared by both facets: opening/continuing a session and running one turn.
/// On `Instance` (not a facet) so the platform `route_message` and the operator `imprint` both reach
/// it.
impl Instance {
    /// Open or continue the session for `conversation`, then run one turn of `inbound` from
    /// `participant` under `template`/`authority`, returning its report and the live buffer it saw
    /// (the buffer the caller's compaction trigger measures). The shared core behind
    /// `Platform::route_message` and `Control::imprint`.
    async fn run_session_turn(
        &self,
        model: &dyn ModelClient,
        routed: &RoutedTurn<'_>,
    ) -> Result<(TurnReport, Vec<TurnView>), InstanceError> {
        // The per-turn observability span (spec §Observability → per-turn spans): wraps the whole
        // turn — session open, the forced catch-up, and the model step loop — so its close carries
        // the turn's wall-clock duration. The result fields (outcome, steps, blocks, prompt tokens)
        // are known only after the turn resolves, so they are recorded into the span below, after
        // the instrumented future completes. Throughput and latency counters are observed here too,
        // covering both the success and error paths in one place.
        let started = std::time::Instant::now();
        let span = tracing::info_span!(
            "turn",
            conversation = ?routed.conversation,
            template = ?routed.template,
            turn_id = tracing::field::Empty,
            outcome = tracing::field::Empty,
            duration_ms = tracing::field::Empty,
            steps = tracing::field::Empty,
            blocks = tracing::field::Empty,
            prompt_tokens = tracing::field::Empty,
        );
        let result = self
            .run_session_turn_inner(model, routed)
            .instrument(span.clone())
            .await;
        let duration = started.elapsed();
        match &result {
            Ok((report, _)) => {
                observe_turn(duration);
                // The outcome is a label ("reply"/"silent"/"max_steps"), never the reply text —
                // traces carry structural identifiers (conversation, turn_id) an operator uses to
                // find the turn's events in the log, not conversational content.
                let outcome = match report.outcome {
                    TurnOutcome::Reply(_) => "reply",
                    TurnOutcome::Silent => "silent",
                    TurnOutcome::MaxStepsExceeded => "max_steps",
                };
                span.record("turn_id", tracing::field::debug(&report.turn_id));
                span.record("outcome", outcome);
                span.record("duration_ms", duration.as_millis() as u64);
                span.record("steps", report.steps);
                span.record("blocks", report.blocks);
                span.record("prompt_tokens", report.prompt_tokens.unwrap_or(0));
            }
            Err(error) => {
                // The cause label distinguishes where the turn failed (model/lua/store/graph); a
                // non-`TurnError` (e.g. an `ensure_session` failure) is `none`.
                let cause = match error {
                    InstanceError::Turn { error, .. } => match error {
                        TurnError::Model(_) => "model",
                        TurnError::Lua(_) => "lua",
                        TurnError::Store(_) => "store",
                        TurnError::Graph(_) => "graph",
                    },
                    _ => "none",
                };
                observe_turn_error("turn", cause, duration);
                span.record("outcome", "error");
                span.record("duration_ms", duration.as_millis() as u64);
            }
        }
        result
    }

    async fn run_session_turn_inner(
        &self,
        model: &dyn ModelClient,
        routed: &RoutedTurn<'_>,
    ) -> Result<(TurnReport, Vec<TurnView>), InstanceError> {
        // `ensure_session` returns the open session as an `Arc`, so the turn holds it across
        // `run_turn().await` without keeping the `sessions` map guard.
        let open = self
            .ensure_session(routed.conversation, routed.present_set, model)
            .await?;
        // The operator's first session runs the imprint interview; once that session has been
        // succeeded by a later one (a lapse, a restart, a compaction), the operator channel uses the
        // ordinary scaffold template — still under operator authority, so it may still write `self` —
        // rather than re-running the imprint's one-time create-a-profile script every turn.
        let template = if matches!(routed.template, PromptTemplateName::Imprint)
            && self
                .engine
                .graph
                .lock()
                .has_earlier_session(routed.conversation, open.id)?
        {
            PromptTemplateName::Scaffold
        } else {
            routed.template
        };
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
            template,
            authority: routed.authority,
            present_set: routed.present_set,
            max_steps,
            block_timeout,
            max_block_attempts,
            capture,
        })
        .await
        .map_err(|error| InstanceError::Turn {
            conversation: Some(routed.conversation),
            error,
        })?;
        Ok((report, buffer))
    }

    /// The lazily-minted async lock serializing `conversation`'s session lifecycle (see [`Instance::
    /// lifecycle`]). Acquired across `ensure_session` and the idle sweep's close, so the close-with-flush
    /// of one session always finishes before the next session for that conversation opens.
    fn lifecycle_lock(&self, conversation: ConversationId) -> Arc<tokio::sync::Mutex<()>> {
        self.lifecycle
            .lock()
            .entry(conversation)
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    }

    /// The features this instance enables — the gate the Lua registration, the API reference, and the
    /// scaffold all read, so the runtime surface, the prompt's description, and the baked guidance
    /// stay in lockstep.
    pub fn features(&self) -> InstanceFeatures {
        self.features
    }

    /// A fresh session VM for a conversation, carrying the MCP projection when servers are connected.
    fn mint_vm(&self, conversation: ConversationId) -> Session {
        match &self.mcp {
            Some(runtime) => Session::with_mcp(
                conversation,
                runtime.host.clone(),
                runtime.catalogue.clone(),
                self.features,
            ),
            None => Session::new(conversation, self.features),
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
    ) -> Result<bool, InstanceError> {
        // The caller holds this conversation's lifecycle lock (see [`Instance::lifecycle`]), so the
        // open-check is reliable here — no other path can be closing this session concurrently. Skip if
        // it is already ended: a path that held the lock before us (the sweep, or the recovery close) has
        // closed it.
        if !self.engine.graph.lock().session_is_open(open.id)? {
            return Ok(false);
        }
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
            .await
            .map_err(|error| InstanceError::Turn {
                conversation: Some(conversation),
                error,
            })?;
            self.engine
                .graph
                .lock()
                .materialize_from(self.engine.store.lock().as_ref())?;
            observe_flush_turn();
        }
        open.vm.shutdown_mcp().await;
        let now = self.engine.clock.now();
        self.engine.store.lock().append(
            now,
            vec![EventPayload::session_ended(conversation, open.id)],
        )?;
        observe_session_closed();
        // Apply the close to the graph so the session reads as `ended`. Without this the `SessionEnded`
        // lands in the log but not the projection, so `open_sessions` keeps returning the session and
        // the idle sweep re-closes it every tick, appending a fresh `SessionEnded` each time.
        self.engine
            .graph
            .lock()
            .materialize_from(self.engine.store.lock().as_ref())?;
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
    ) -> Result<Arc<OpenSession>, InstanceError> {
        // Serialize this conversation's lifecycle: hold its lock across the whole recover/close/open so an
        // idle-sweep close already in flight for it finishes first — its flush's writes are then in the
        // graph the new session's brief reads — and so the close and the next open never interleave.
        let lifecycle = self.lifecycle_lock(conversation);
        let _lifecycle = lifecycle.lock().await;

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
        // background driver ([`Instance::run_scheduler`]) fires continuously on a timer; this catch-up
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
        observe_session_opened();
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
            let surface_count = drained.entries.len();
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
                payloads.push(EventPayload::scheduled_item_surfaced(
                    entry_id, memory, id, now,
                ));
            }
            self.engine.store.lock().append(now, payloads)?;
            observe_wakeups_surfaced(surface_count);
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
    fn resolve_or_mint_operator(&self) -> Result<MemoryId, InstanceError> {
        let operator = MemoryName::from(NamespacedMemoryName::operator());
        if let Some(memory) = self.engine.graph.lock().memory_by_name(&operator)? {
            return Ok(memory.id);
        }
        let id = MemoryId::generate();
        let now = self.engine.clock.now();
        self.engine
            .store
            .lock()
            .append(now, vec![EventPayload::memory_created(id, operator)])?;
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
    fn fire_due_now(&self, now: Timestamp) -> Result<usize, InstanceError> {
        let fired = {
            let graph = self.engine.graph.lock();
            scheduler::fire_due(self.engine.store.lock().as_mut(), &graph, now)?
        };
        if fired > 0 {
            observe_wakeups_fired(fired);
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
    /// on the shared `Arc<Instance>` until `shutdown` resolves.
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
                            observe_worker_error("scheduler");
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
    fn snapshot_if_due(&self, schedule: &SnapshotSchedule) -> Result<bool, InstanceError> {
        let head = self.engine.graph.lock().head()?;
        let last = snapshot::latest(&schedule.dir)
            .map_err(|error| InstanceError::Snapshot(error.to_string()))?
            .map_or(0, |(_, head)| head.0);
        if head.0.saturating_sub(last) < schedule.min_new_events {
            return Ok(false);
        }
        let wrote = self.snapshot(&schedule.dir)?.is_some();
        if wrote {
            snapshot::prune(&schedule.dir, schedule.keep)
                .map_err(|error| InstanceError::Snapshot(error.to_string()))?;
        }
        Ok(wrote)
    }

    /// The background snapshotter: on each `check_interval` tick, snapshot the graph if activity has
    /// accrued ([`Instance::snapshot_if_due`]), stopping on the same shutdown signal as the scheduler.
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
    pub async fn index_catch_up(&self) -> Result<usize, InstanceError> {
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
    pub async fn reembed_if_embedding_model_changed(&self) -> Result<bool, InstanceError> {
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
                    vec![EventPayload::embedding_model_changed(recorded, configured)],
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
                        Err(error) => {
                            observe_worker_error("indexer");
                            tracing::error!(%error, "indexer: catch-up failed")
                        }
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
    pub async fn describe_catch_up(&self, model: &dyn ModelClient) -> Result<usize, InstanceError> {
        // Held across the catch-up so a concurrent pass waits, then reads the already-advanced cursor
        // and no-ops, rather than re-describing the same memories.
        let _guard = self.describe_guard.lock().await;
        let cursor = *self.describer_cursor.lock();
        let (advanced, count) = run_describe_catch_up(&self.engine, model, cursor).await?;
        *self.describer_cursor.lock() = advanced;
        Ok(count)
    }

    /// Seed the describer's cursor to log-head, treating everything written so far as described. Called
    /// at boot and at agent creation so the genesis-seeded `self` (which has no description yet) is not
    /// regenerated by a synchronous catch-up before any real content is written.
    pub(crate) fn baseline_describer_cursor(&self) -> Result<(), InstanceError> {
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
                        Err(error) => {
                            observe_worker_error("describe");
                            tracing::error!(%error, "describer: catch-up failed")
                        }
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
    pub async fn adjudicate_catch_up(
        &self,
        model: &dyn ModelClient,
    ) -> Result<usize, InstanceError> {
        let _guard = self.adjudicate_guard.lock().await;
        let cursor = *self.adjudicator_cursor.lock();
        let (advanced, count) = run_adjudicate_catch_up(&self.engine, model, cursor).await?;
        *self.adjudicator_cursor.lock() = advanced;
        Ok(count)
    }

    /// Seed the adjudicator's cursor to log-head, treating every proposal so far as already adjudicated.
    /// Called at boot and at agent creation, like the describer's, so a restart does not re-weigh old
    /// proposals.
    pub(crate) fn baseline_adjudicator_cursor(&self) -> Result<(), InstanceError> {
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
                        Err(error) => {
                            observe_worker_error("adjudicate");
                            tracing::error!(%error, "adjudicator: catch-up failed")
                        }
                    }
                }
                _ = &mut shutdown => break,
            }
        }
        tracing::info!("adjudicator stopped");
    }

    /// Catch link inference up to the log off the hot path (spec §Write path → link inference):
    /// identify relationships implicit in every memory whose content changed since the cursor,
    /// advancing it. Driven on a timer by the served runtime and explicitly by tests and the eval
    /// harness. Returns how many memories it considered.
    pub async fn link_inference_catch_up(
        &self,
        model: &dyn ModelClient,
    ) -> Result<usize, InstanceError> {
        let _guard = self.link_inference_guard.lock().await;
        let cursor = *self.link_inference_cursor.lock();
        let (advanced, count) = run_link_inference_catch_up(&self.engine, model, cursor).await?;
        *self.link_inference_cursor.lock() = advanced;
        Ok(count)
    }

    /// Seed the link-inference pass's cursor to log-head, treating every relationship so far as
    /// already inferred. Called at boot and at agent creation, like the describer's and adjudicator's,
    /// so a restart does not re-infer over old content.
    pub(crate) fn baseline_link_inference_cursor(&self) -> Result<(), InstanceError> {
        *self.link_inference_cursor.lock() = self.engine.store.lock().head()?;
        Ok(())
    }

    /// The background link-inference pass: on each tick, infer relationships off the hot path.
    /// Idempotent and cursor-resumed, so an idle tick is cheap; a failure is logged, not fatal — a
    /// memory stays un-inferred until the next tick.
    pub async fn run_link_inference(
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
                    match self.link_inference_catch_up(model.as_ref()).await {
                        Ok(considered) if considered > 0 => {
                            tracing::debug!(considered, "link inference inferred relationships")
                        }
                        Ok(_) => {}
                        Err(error) => {
                            observe_worker_error("link_inference");
                            tracing::error!(%error, "link inference: catch-up failed")
                        }
                    }
                }
                _ = &mut shutdown => break,
            }
        }
        tracing::info!("link inference stopped");
    }

    /// Close-with-flush every session idle past the gap — the proactive consolidation that bounds how
    /// long a conversation's working state can sit unflushed: the no-loss guarantee for a conversation
    /// never messaged again (a passive exit or a restart leaves its session open in the log, and only
    /// the message path resolves a session that *is* messaged). A live session's touched last-activity
    /// is authoritative; a log-only one's comes from its last recorded turn. The session is claimed in
    /// the map (reconstructed if only in the log) and then taken back out with an atomic `remove` — the
    /// single point that dedupes a concurrent message's own close of the same session, so it is closed
    /// exactly once. Returns how many sessions it closed. Driven on a timer by [`Instance::run_sweeper`];
    /// also callable directly to sweep once on demand.
    pub async fn sweep_idle_sessions(
        &self,
        model: &dyn ModelClient,
    ) -> Result<usize, InstanceError> {
        let now = self.engine.clock.now();
        let idle_gap_ms = Settings::from_store(self.engine.store.lock().as_ref())?
            .compaction
            .idle_gap_seconds
            .saturating_mul(1_000);
        let mut closed = 0;
        // Bind the list first so the graph guard drops before the per-session flush `.await` below.
        let open = self.engine.graph.lock().open_sessions()?;
        for (conversation, recovered) in open {
            let live_activity = self
                .sessions
                .lock()
                .get(&conversation)
                .map(|open| open.last_activity_millis());
            let last_activity_ms = match live_activity {
                Some(ms) => ms,
                None => buffer_turns(
                    self.engine.store.lock().as_ref(),
                    conversation,
                    recovered.start_seq,
                )?
                .last()
                .map_or(recovered.started_at, |turn| turn.recorded_at)
                .as_millis(),
            };
            if now.as_millis() - last_activity_ms <= idle_gap_ms {
                continue;
            }
            // Hold the conversation's lifecycle lock across the close, so a message arriving mid-flush
            // waits in `ensure_session` rather than opening a new session before this flush lands.
            let lifecycle = self.lifecycle_lock(conversation);
            let _lifecycle = lifecycle.lock().await;
            // Re-validate under the lock: a message that arrived since the candidate list was captured may
            // have closed this session and opened a newer one, which must not be closed here.
            if !self.engine.graph.lock().session_is_open(recovered.id)? {
                continue;
            }
            // Close this specific candidate: reuse the live handle if it is this session, else mint one.
            let stale = {
                let mut sessions = self.sessions.lock();
                if sessions
                    .get(&conversation)
                    .is_some_and(|s| s.id == recovered.id)
                {
                    sessions
                        .remove(&conversation)
                        .expect("present under the lock")
                } else {
                    Arc::new(OpenSession {
                        id: recovered.id,
                        vm: self.mint_vm(conversation),
                        brief: recovered.brief,
                        started_at: recovered.started_at,
                        last_activity: AtomicI64::new(last_activity_ms),
                        start_seq: recovered.start_seq,
                    })
                }
            };
            self.flush_and_end(conversation, stale.as_ref(), model)
                .await?;
            closed += 1;
        }
        Ok(closed)
    }

    /// The background idle-sweep driver (the no-loss timer): on each tick, close-with-flush every
    /// session idle past the gap, so a conversation's working state is consolidated even if it is never
    /// messaged again. Long-lived; a sweep failure is logged, never propagated.
    pub async fn run_sweeper(
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
                    match self.sweep_idle_sessions(model.as_ref()).await {
                        Ok(closed) if closed > 0 => {
                            tracing::info!(closed, "idle sweep closed stale sessions")
                        }
                        Ok(_) => {}
                        Err(error) => {
                            observe_worker_error("sweep");
                            tracing::error!(%error, "idle sweep failed")
                        }
                    }
                }
                _ = &mut shutdown => break,
            }
        }
        tracing::info!("idle sweep driver stopped");
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

/// The live session backing a conversation (runtime state, see [`Instance::sessions`]). Held behind an
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

/// An instance-side failure, delegating its message to the underlying error.
#[derive(Debug)]
pub enum InstanceError {
    Store(StoreError),
    Graph(GraphError),
    /// A turn (the agent loop) failed while routing a message. `conversation` is `Some` for a
    /// routed turn or flush (the common case) and `None` for a background catch-up (describe/
    /// adjudicate), which spans all conversations rather than one.
    Turn {
        conversation: Option<ConversationId>,
        error: TurnError,
    },
    /// Connecting the MCP servers failed (a probe-level hard error, e.g. a stale `allow`/`deny`).
    Mcp(crate::mcp::McpError),
    /// Writing a graph snapshot failed (creating the directory, or the `VACUUM INTO` itself).
    Snapshot(String),
    /// Catching the vector index up to the log failed (embedding, the vector store, or the log read).
    Index(IndexError),
    /// A semantic search failed (the graph projection or the vector index).
    Search(SearchError),
    /// An operator Lua console block failed at the VM level (a script error reaches the operator as
    /// a result, not this; this is an infrastructure failure running the block). `conversation` is the
    /// dedicated console conversation, packed at the call boundary.
    Lua {
        conversation: Option<ConversationId>,
        error: crate::agent::lua::LuaError,
    },
}

impl std::fmt::Display for InstanceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InstanceError::Store(error) => write!(f, "instance (store): {error}"),
            InstanceError::Graph(error) => write!(f, "instance (graph): {error}"),
            InstanceError::Turn {
                conversation,
                error,
            } => match conversation {
                Some(id) => write!(f, "instance (turn {}): {error}", id.0),
                None => write!(f, "instance (turn): {error}"),
            },
            InstanceError::Mcp(error) => write!(f, "instance (mcp): {error}"),
            InstanceError::Snapshot(message) => write!(f, "instance (snapshot): {message}"),
            InstanceError::Index(error) => write!(f, "instance (index): {error}"),
            InstanceError::Search(error) => write!(f, "instance (search): {error}"),
            InstanceError::Lua {
                conversation,
                error,
            } => match conversation {
                Some(id) => write!(f, "instance (lua {}): {error}", id.0),
                None => write!(f, "instance (lua): {error}"),
            },
        }
    }
}

impl std::error::Error for InstanceError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            InstanceError::Store(error) => Some(error),
            InstanceError::Graph(error) => Some(error),
            InstanceError::Turn { error, .. } => Some(error),
            InstanceError::Mcp(error) => Some(error),
            InstanceError::Snapshot(_) => None,
            InstanceError::Index(error) => Some(error),
            InstanceError::Search(error) => Some(error),
            InstanceError::Lua { error, .. } => Some(error),
        }
    }
}

impl From<SearchError> for InstanceError {
    fn from(error: SearchError) -> Self {
        InstanceError::Search(error)
    }
}

impl From<IndexError> for InstanceError {
    fn from(error: IndexError) -> Self {
        InstanceError::Index(error)
    }
}

impl From<crate::mcp::McpError> for InstanceError {
    fn from(error: crate::mcp::McpError) -> Self {
        InstanceError::Mcp(error)
    }
}

impl From<StoreError> for InstanceError {
    fn from(error: StoreError) -> Self {
        InstanceError::Store(error)
    }
}

impl From<GraphError> for InstanceError {
    fn from(error: GraphError) -> Self {
        InstanceError::Graph(error)
    }
}

// Identity and brief resolution fail only into store/graph errors, so they map onto the existing
// variants rather than widening the enum; the agent loop's richer `TurnError` keeps its own.
impl From<IdentityError> for InstanceError {
    fn from(error: IdentityError) -> Self {
        match error {
            IdentityError::Store { source, .. } => InstanceError::Store(source),
            IdentityError::Graph { source, .. } => InstanceError::Graph(source),
        }
    }
}

impl From<BriefError> for InstanceError {
    fn from(error: BriefError) -> Self {
        match error {
            BriefError::Graph(error) => InstanceError::Graph(error),
        }
    }
}

impl From<SchedulerError> for InstanceError {
    fn from(error: SchedulerError) -> Self {
        match error {
            SchedulerError::Store(error) => InstanceError::Store(error),
            SchedulerError::Graph(error) => InstanceError::Graph(error),
        }
    }
}

impl From<TurnError> for InstanceError {
    fn from(error: TurnError) -> Self {
        InstanceError::Turn {
            conversation: None,
            error,
        }
    }
}

impl From<crate::agent::lua::LuaError> for InstanceError {
    fn from(error: crate::agent::lua::LuaError) -> Self {
        InstanceError::Lua {
            conversation: None,
            error,
        }
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
    ) -> Instance {
        Instance::with_retrieval(
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
                vec![EventPayload::memory_description_regenerated(
                    mem,
                    "an avid climber".to_owned(),
                    None,
                )],
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
    async fn the_idle_sweep_closes_a_session_once_not_every_tick() {
        // Regression: `flush_and_end` must apply its `SessionEnded` to the graph, not only append it.
        // Otherwise `open_sessions` keeps returning the closed session and the sweep re-closes it every
        // tick — the live-instance "the session ended right after my message" loop.
        let conversation = ConversationId::generate();
        let session = SessionId::generate();
        let mut store = MemoryStore::new();
        store
            .append(
                Timestamp::from_millis(1_000),
                vec![EventPayload::SessionStarted {
                    conversation,
                    id: session,
                    participants: vec![],
                    started_at: Timestamp::from_millis(1_000),
                    seeded_from_turn: None,
                    brief: "brief".to_owned(),
                }],
            )
            .unwrap();

        let mut graph = Graph::open_in_memory().unwrap();
        graph.materialize_from(&store).unwrap();
        let clock = ManualClock::new(Timestamp::from_millis(2_000));
        let server = Instance::new(Box::new(store), graph, Box::new(clock.clone()));
        assert_eq!(
            server.engine.graph.lock().open_sessions().unwrap().len(),
            1,
            "the session starts open"
        );

        // Past the idle gap; the session has no content turns, so the close skips the flush turn and
        // never calls the model.
        clock.advance_millis(7_200_000);
        let model = crate::model::ScriptedModel::new([]);

        assert_eq!(
            server.sweep_idle_sessions(&model).await.unwrap(),
            1,
            "the first sweep closes the idle session"
        );
        assert!(
            server
                .engine
                .graph
                .lock()
                .open_sessions()
                .unwrap()
                .is_empty(),
            "the close must reach the graph so the session reads as ended"
        );
        assert_eq!(
            server.sweep_idle_sessions(&model).await.unwrap(),
            0,
            "a second sweep must not re-close it"
        );

        // The close is recorded exactly once — no repeated `SessionEnded`.
        let ends = server
            .engine
            .store
            .lock()
            .read_from(Seq::ZERO)
            .unwrap()
            .iter()
            .filter(|event| matches!(event.payload, EventPayload::SessionEnded { .. }))
            .count();
        assert_eq!(ends, 1, "the session is ended once, never re-closed");

        // And `flush_and_end` itself is idempotent: invoked again on the now-closed session (as a stale
        // sweep candidate would), it skips rather than appending a second close.
        let stale = Arc::new(OpenSession {
            id: session,
            vm: server.mint_vm(conversation),
            brief: "brief".to_owned(),
            started_at: Timestamp::from_millis(1_000),
            last_activity: AtomicI64::new(1_000),
            start_seq: Seq(1),
        });
        assert!(
            !server
                .flush_and_end(conversation, &stale, &model)
                .await
                .unwrap(),
            "flush_and_end on an already-ended session is a no-op"
        );
        let ends_after = server
            .engine
            .store
            .lock()
            .read_from(Seq::ZERO)
            .unwrap()
            .iter()
            .filter(|event| matches!(event.payload, EventPayload::SessionEnded { .. }))
            .count();
        assert_eq!(ends_after, 1, "no second close was appended");
    }

    #[tokio::test]
    async fn concurrent_closes_of_one_session_record_a_single_end() {
        // A close runs a flush — a model call lasting seconds — before recording `SessionEnded`. In that
        // window the idle sweep and the message-driven recovery path both reach the close for one session.
        // Both hold the conversation's lifecycle lock; serialized through it, the first closes and the
        // second sees the session already ended and skips — exactly one `SessionEnded`, not two.
        let conversation = ConversationId::generate();
        let session = SessionId::generate();
        let mut store = MemoryStore::new();
        store
            .append(
                Timestamp::from_millis(1_000),
                vec![EventPayload::SessionStarted {
                    conversation,
                    id: session,
                    participants: vec![],
                    started_at: Timestamp::from_millis(1_000),
                    seeded_from_turn: None,
                    brief: "brief".to_owned(),
                }],
            )
            .unwrap();
        let mut graph = Graph::open_in_memory().unwrap();
        graph.materialize_from(&store).unwrap();
        let server = Instance::new(
            Box::new(store),
            graph,
            Box::new(ManualClock::new(Timestamp::from_millis(2_000))),
        );

        let open = Arc::new(OpenSession {
            id: session,
            vm: server.mint_vm(conversation),
            brief: "brief".to_owned(),
            started_at: Timestamp::from_millis(1_000),
            last_activity: AtomicI64::new(1_000),
            start_seq: Seq(1),
        });
        let model = crate::model::ScriptedModel::new([]);
        let lifecycle = server.lifecycle_lock(conversation);
        let (a, b) = tokio::join!(
            async {
                let _held = lifecycle.lock().await;
                server.flush_and_end(conversation, &open, &model).await
            },
            async {
                let _held = lifecycle.lock().await;
                server.flush_and_end(conversation, &open, &model).await
            },
        );
        a.unwrap();
        b.unwrap();

        let ends = server
            .engine
            .store
            .lock()
            .read_from(Seq::ZERO)
            .unwrap()
            .iter()
            .filter(|event| matches!(event.payload, EventPayload::SessionEnded { .. }))
            .count();
        assert_eq!(
            ends, 1,
            "two concurrent closes record exactly one SessionEnded"
        );
        assert!(
            !server.engine.graph.lock().session_is_open(session).unwrap(),
            "the session is left ended"
        );
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
        use crate::{ids::Namespace, store::SqliteStore, vector::SqliteVectorIndex};

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
                        EventPayload::memory_created(mem, Namespace::Topic.with_name("x")),
                        EventPayload::memory_description_regenerated(
                            mem,
                            "an avid climber".to_owned(),
                            None,
                        ),
                    ],
                )
                .unwrap();
            let server = Instance::with_retrieval(
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
            let mut server = Instance::with_retrieval(
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

#[cfg(test)]
mod observability_tests {
    //! The metrics helpers observe a conversational turn and its model calls (spec §Observability →
    //! metrics). Driven with a scripted model so no GPU is needed; a thread-local recorder
    //! (`set_default_local_recorder`) keeps each test isolated — no global recorder, no cross-test
    //! pollution. The per-turn span's step/block counts are exercised by the `TurnReport` counting
    //! test in `tests/agent.rs`; the span itself is surfaced by `init_tracing`'s `FmtSpan::CLOSE`.
    use super::*;
    use crate::{
        ConversationLocator,
        clock::ManualClock,
        metrics::{LATENCY_BUCKETS, describe},
        model::{Completion, ScriptedModel},
        time::Timestamp,
    };

    fn born_server() -> Instance {
        let server =
            Instance::in_memory(Box::new(ManualClock::new(Timestamp::from_millis(0)))).unwrap();
        server
            .control()
            .create_agent(&crate::SeedSelf {
                agent_name: "Kestrel".to_owned(),
                persona: "An assistant.".to_owned(),
                seed_entries: vec![],
            })
            .unwrap();
        server
    }

    #[tokio::test]
    async fn a_turn_observes_its_metrics() {
        let recorder = metrics_exporter_prometheus::PrometheusBuilder::new()
            .set_buckets(LATENCY_BUCKETS)
            .unwrap()
            .build_recorder();
        let handle = recorder.handle();
        let _guard = metrics::set_default_local_recorder(&recorder);
        describe();
        let server = born_server();
        let model = ScriptedModel::new([Completion::Reply("Hi there.".to_owned())]);
        server
            .platform()
            .route_message(
                &model,
                &ConversationLocator::new("discord", "general"),
                "dave",
                "hello",
                &["dave"],
            )
            .await
            .unwrap();
        server.control().refresh_gauges().unwrap();
        let text = handle.render();
        assert!(
            text.contains("zuihitsu_turns_total 1\n"),
            "one turn observed"
        );
        assert!(
            text.contains("zuihitsu_model_calls_total 1\n"),
            "the turn's step was observed at the chokepoint"
        );
        assert!(text.contains("zuihitsu_sessions_opened_total 1\n"));
        assert!(
            text.contains("zuihitsu_sessions_active 1\n"),
            "session stays open"
        );
        // The agent-state gauges were refreshed from the graph.
        assert!(text.contains("zuihitsu_memory_count"));
        // The describer was caught up to the pre-turn head, so the turn's own writes leave a lag.
        assert!(
            text.contains("zuihitsu_describer_lag_seq ")
                && !text.contains("zuihitsu_describer_lag_seq 0\n"),
            "the turn's writes lag the describer cursor"
        );
    }

    #[tokio::test]
    async fn model_call_tokens_accumulate_from_usage() {
        // A scripted step that reports token usage feeds the cumulative token counters.
        let recorder = metrics_exporter_prometheus::PrometheusBuilder::new()
            .set_buckets(LATENCY_BUCKETS)
            .unwrap()
            .build_recorder();
        let handle = recorder.handle();
        let _guard = metrics::set_default_local_recorder(&recorder);
        describe();
        let server = born_server();
        let model = ScriptedModel::with_responses([crate::model::GenerateResponse {
            completion: Completion::Reply("Hi there.".to_owned()),
            usage: crate::model::Usage {
                prompt_tokens: Some(120),
                completion_tokens: Some(30),
                total_tokens: Some(150),
            },
            reasoning: None,
            finish_reason: Some("stop".to_owned()),
        }]);
        server
            .platform()
            .route_message(
                &model,
                &ConversationLocator::new("discord", "general"),
                "dave",
                "hello",
                &["dave"],
            )
            .await
            .unwrap();
        let text = handle.render();
        assert!(text.contains("zuihitsu_model_prompt_tokens_total 120\n"));
        assert!(text.contains("zuihitsu_model_completion_tokens_total 30\n"));
    }
}
