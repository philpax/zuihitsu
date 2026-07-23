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
mod drivers;
mod error;
mod platform;
mod session;
#[cfg(test)]
mod tests;
mod workers;

pub use drivers::CheckpointTrigger;
pub use error::InstanceError;
use session::{OpenSession, RoutedTurn, TailSeed, carryover_tail};

pub use control::{
    Arbitration, ContextEntry, Control, DesignateOutcome, LuaConsoleOutcome, MergeProposal,
    ModelCall, RetractOutcome, SelfEditOutcome, UnmergeOutcome,
};
pub use platform::{
    LinkError, LinkNode, MessageInput, ParticipantAttribute, Platform, ProjectOutcome, RosterResync,
};

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use parking_lot::Mutex;
use tokio::sync::Semaphore;

use crate::{
    InstanceFeatures,
    agent::{
        McpCatalogue,
        genesis::{self, GenesisStatus},
    },
    clock::Clock,
    engine::Engine,
    graph::Graph,
    ids::{ConversationId, MemoryId, Seq},
    instance::workers::MaintenanceStart,
    mcp::{McpHost, McpServerConfig},
    memory::search::{SearchHit, SearchQuery, search as rank_search},
    metrics::observe_search,
    model::{ModelClient, embed::Embedder, index::IndexError},
    settings::{ConcurrencySettings, Settings},
    snapshot,
    store::{MemoryStore, Store},
    vector::VectorIndex,
    web::{WebClient, WebFetcher},
};

pub struct Instance {
    // The store, graph, and clock bundled behind one shared [`Engine`], so a turn shares them with a
    // single pointer bump and the Lua block API can hold a `'static` handle across `eval_async`. The
    // instance is still the single writer; the engine's mutexes serialize access rather than admit a
    // second writer. See [`Engine`] for the graph-before-store lock-ordering rule.
    engine: Arc<Engine>,
    /// The live session map and its lifecycle/carryover state, grouped as [`SessionStore`].
    sessions: SessionStore,
    /// The per-conversation turn ledger (spec §Concurrency → per-conversation supersession): the
    /// serialization slot and arrival-epoch signal a room's turns share, so batches for one
    /// conversation serialize and a newer batch cooperatively supersedes an in-flight turn. Pure
    /// runtime state — never logged; an agent restart drops it and the next batch rebuilds a
    /// conversation's entry on first contact.
    turns: TurnLedger,
    /// The off-hot-path synthesis cursors and their serialization guards, grouped as
    /// [`BackgroundPasses`]. Each cursor tracks how far a background pass has progressed; its guard
    /// serializes that pass against the explicit catch-up.
    passes: BackgroundPasses,
    /// The concurrent-stream limit (spec §Concurrency): a permit is held for each in-flight inbound
    /// message's whole handling, so no more than `max_concurrent_streams` turns crowd the shared
    /// model at once; further streams queue. Sized from settings at construction (a change takes
    /// effect on restart).
    streams: Semaphore,
    /// The MCP host and the catalogue probed from it at [`Instance::connect_mcp`] — `None` until then.
    /// Each session opened while it is set gets the `mcp.<server>.*` projection over the same catalogue.
    mcp: Option<McpRuntime>,
    /// The web fetcher and its Markdown cap, set by [`Instance::connect_web`] — `None` until then.
    /// Each session opened while it is set gets the `web.markdown` projection over it (gated on the
    /// `browsing` feature).
    web: Option<WebClient>,
    /// Which API features this instance enables — gates the Lua functions installed per block, the
    /// API reference rendered into the system prompt, and (at genesis) the scaffold dotpoints. Set
    /// at construction, before genesis, so the baked scaffold reflects it; defaults to all-on.
    features: InstanceFeatures,
    /// The configured model's context window, in tokens, set by the serving host from `[model]`
    /// config. `None` for an in-memory or model-less instance. Genesis derives the agent's initial
    /// compaction budget from it (a fraction of the window); see [`Control::create_agent`].
    model_context_length: Option<u32>,
}

/// The off-hot-path synthesis cursors and their serialization guards. Each cursor tracks how far
/// a background pass has progressed; its guard serializes that pass against the explicit catch-up.
/// The link-inference cursor is re-seeded to log-head at boot, treating already-written state as
/// processed so a restart does not re-run that pass. The describer keeps no cursor — its backlog is
/// the graph's log-derived per-memory described-state, so a pending describe backlog persists across a
/// restart rather than being reset at boot (spec §Write path).
pub(crate) struct BackgroundPasses {
    /// The link-inference pass's cursor: the log seq through which implicit relationships have been
    /// inferred and linked. Its own background pass (and the explicit `link_inference_catch_up`)
    /// advances it as it extracts relationships off the hot path (spec §Write path → link inference).
    /// Re-seeded to log-head at boot, like the describer's.
    link_inference_cursor: Mutex<Seq>,
    /// Serializes describe-catch-up passes — the narrow force-before-brief one (a new session opening)
    /// and the background timer one. Held per memory rather than across a whole pass, so a session
    /// open's narrow pass interleaves with a long background backlog; each memory's staleness is
    /// re-checked under it, so two passes never redescribe the same memory for the same change.
    describe_guard: tokio::sync::Mutex<()>,
    /// As `describe_guard`, serializing the link-inference catch-up.
    link_inference_guard: tokio::sync::Mutex<()>,
    /// The consolidation pass's cursor: the log seq through which entries have been considered for
    /// consolidation. Re-seeded to log-head at boot, like the link-inference cursor.
    consolidation_cursor: Mutex<Seq>,
    /// Serializes consolidation sweeps so two do not overlap.
    consolidation_guard: tokio::sync::Mutex<()>,
    /// The canonicalize pass's cursor: the log seq through which platform stubs have been considered.
    canonicalize_cursor: Mutex<Seq>,
    /// Serializes canonicalize sweeps.
    canonicalize_guard: tokio::sync::Mutex<()>,
    /// The link-cleanup pass's cursor: the log seq through which entries have been considered for
    /// link-redundancy cleanup.
    link_cleanup_cursor: Mutex<Seq>,
    /// Serializes link-cleanup sweeps.
    link_cleanup_guard: tokio::sync::Mutex<()>,
}

mod session_store;
mod turn_ledger;

pub(crate) use session_store::SessionStore;
pub(crate) use turn_ledger::TurnLedger;

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
            sessions: SessionStore::new(),
            turns: TurnLedger::new(),
            passes: BackgroundPasses::new(Seq::ZERO),
            streams,
            mcp: None,
            web: None,
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

    /// Attach the web fetcher backing `web.markdown`, projected into every session opened from now on
    /// (gated on the `browsing` feature). Called once after construction by whoever drives serving —
    /// the serving host wires the real [`HttpFetcher`], tests and the eval inject a fake. Idempotent
    /// re-wiring is fine; the last fetcher set wins.
    pub fn connect_web(&mut self, fetcher: Arc<dyn WebFetcher>, max_markdown_chars: usize) {
        self.web = Some(WebClient::new(fetcher, max_markdown_chars));
    }

    /// Subscribe to committed events — the store's live feed, which the control surface's push
    /// channel (`GET /control/events/stream`) fans out to its viewers.
    pub fn subscribe_events(&self) -> zuihitsu_core::store::Subscription {
        self.engine.store.lock().subscribe()
    }

    /// Subscribe to the ephemeral turn-progress feed (see [`crate::engine::ProgressFeed`]): the
    /// token-by-token deliberation of in-flight turns, never stored and never replayed.
    pub fn subscribe_progress(
        &self,
    ) -> tokio::sync::broadcast::Receiver<zuihitsu_core::progress::TurnProgress> {
        self.engine.progress.subscribe()
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
        // A born agent picks up any template names this build introduced after its genesis —
        // additive only, so operator-curated registrations are never touched (see
        // `genesis::reconcile_new_templates`). Before materialization, so the graph applies the
        // registrations in the same catch-up.
        if genesis::status(self.engine.store.lock().as_ref())? == GenesisStatus::Complete {
            genesis::reconcile_new_templates(
                self.engine.store.lock().as_mut(),
                self.engine.clock.as_ref(),
                &self.features,
            )?;
        }
        let applied = self
            .engine
            .graph
            .lock()
            .materialize_from(self.engine.store.lock().as_ref())?;
        // Seed the link-inference cursor to log-head: state written before this boot is treated as
        // already processed, so a restart does not re-run that pass (spec §Write path). The describer
        // needs no seeding — its backlog is the graph's log-derived per-memory described-state, so a
        // pre-shutdown backlog survives the restart and is caught up here.
        self.passes.reseed(self.engine.store.lock().head()?);
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

    // Background-pass facade methods: delegate to `self.passes` with `&self.engine`. These preserve
    // the public API consumed by the http server, tests, and the control facet.

    pub async fn index_catch_up(&self) -> Result<usize, InstanceError> {
        self.passes.index_catch_up(&self.engine).await
    }

    pub async fn reembed_if_embedding_model_changed(&self) -> Result<bool, InstanceError> {
        self.passes
            .reembed_if_embedding_model_changed(&self.engine)
            .await
    }

    pub async fn describe_catch_up(&self, model: &dyn ModelClient) -> Result<usize, InstanceError> {
        self.passes.describe_catch_up(&self.engine, model).await
    }

    pub async fn describe_catch_up_for(
        &self,
        model: &dyn ModelClient,
        ids: &[MemoryId],
    ) -> Result<usize, InstanceError> {
        self.passes
            .describe_catch_up_for(&self.engine, model, ids)
            .await
    }

    pub async fn link_inference_catch_up(
        &self,
        model: &dyn ModelClient,
    ) -> Result<usize, InstanceError> {
        self.passes
            .link_inference_catch_up(&self.engine, model)
            .await
    }

    pub(crate) fn baseline_link_inference_cursor(&self) -> Result<(), InstanceError> {
        self.passes.baseline_link_inference_cursor(&self.engine)
    }

    /// Seed the maintenance pass cursors to log-head, so a restart does not re-process old content.
    pub(crate) fn baseline_maintenance_cursors(&self) -> Result<(), InstanceError> {
        self.passes.baseline_maintenance_cursors(&self.engine)
    }

    /// Drive the consolidation pass on demand — the CLI/console entry point. Returns how many memories
    /// were considered. Unlike the timer driver (which resumes from the incremental cursor), the manual
    /// path sweeps the whole log from the start ([`MaintenanceStart::FromStart`]): a fresh instance
    /// seeds the cursor to log-head at boot, so an incremental manual pass would be a no-op that defeats
    /// its purpose. The pass is idempotent, so a full re-sweep is safe.
    pub async fn consolidation_catch_up(
        &self,
        model: &dyn ModelClient,
    ) -> Result<usize, InstanceError> {
        self.passes
            .consolidation_catch_up(&self.engine, model, MaintenanceStart::FromStart)
            .await
    }

    /// Drive the canonical-profile pass on demand. Returns how many stubs were considered. Sweeps from
    /// the start of the log, like [`Instance::consolidation_catch_up`] — see it for the timer/manual
    /// asymmetry.
    pub async fn canonicalize_catch_up(
        &self,
        model: &dyn ModelClient,
    ) -> Result<usize, InstanceError> {
        self.passes
            .canonicalize_catch_up(&self.engine, model, MaintenanceStart::FromStart)
            .await
    }

    /// Drive the link-redundant entry cleanup pass on demand. Returns how many memories were considered.
    /// Sweeps from the start of the log, like [`Instance::consolidation_catch_up`] — see it for the
    /// timer/manual asymmetry.
    pub async fn link_cleanup_catch_up(
        &self,
        model: &dyn ModelClient,
    ) -> Result<usize, InstanceError> {
        self.passes
            .link_cleanup_catch_up(&self.engine, model, MaintenanceStart::FromStart)
            .await
    }

    /// The lazily-minted async lock serializing `conversation`'s session lifecycle. Delegates to
    /// [`SessionStore::lifecycle_lock`]; kept on `Instance` because tests reach it through
    /// `server.lifecycle_lock(conversation)`.
    pub(crate) fn lifecycle_lock(
        &self,
        conversation: ConversationId,
    ) -> Arc<tokio::sync::Mutex<()>> {
        self.sessions.lifecycle_lock(conversation)
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
