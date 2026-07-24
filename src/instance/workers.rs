//! The background catch-up workers: the indexer, describer, and link-inference pass.
//! Each has a synchronous `catch_up` (driven explicitly by tests) and a
//! background `run_*` loop (driven on a timer by the served runtime). All are cursor-resumed and
//! idempotent, so an idle tick is cheap.

use std::{future::Future, sync::Arc, time::Duration};

use parking_lot::Mutex;

use crate::{
    InstanceError,
    agent::{
        maintenance, run_describe_catch_up, run_describe_catch_up_for, run_link_inference_catch_up,
    },
    engine::Engine,
    event::{EventPayload, EventSource},
    ids::{MemoryId, MemoryName, Seq},
    instance::{BackgroundPasses, Instance},
    metrics::{observe_describe_priority_escape, observe_worker_error},
    model::{
        ModelArbiter, ModelClient,
        index::{IndexError, apply_batch, embed_batch},
    },
    settings::Settings,
};

/// Where a maintenance pass begins its sweep. The timer-driven driver resumes from the pass's stored
/// incremental cursor; an on-demand invocation (CLI or console) sweeps the whole log from the start,
/// since a fresh instance seeds every cursor to log-head at boot, which would make an incremental
/// manual pass a no-op. The passes are idempotent, so a full re-sweep is safe.
#[derive(Clone, Copy)]
pub(crate) enum MaintenanceStart {
    /// Resume from the stored incremental cursor — the timer path.
    Cursor,
    /// Sweep the whole log from the start — the on-demand backfill path.
    FromStart,
}

impl MaintenanceStart {
    /// The seq to begin from: the stored cursor for [`MaintenanceStart::Cursor`], or [`Seq::ZERO`] for
    /// [`MaintenanceStart::FromStart`].
    fn cursor(self, stored: &Mutex<Seq>) -> Seq {
        match self {
            MaintenanceStart::Cursor => *stored.lock(),
            MaintenanceStart::FromStart => Seq::ZERO,
        }
    }
}

impl BackgroundPasses {
    /// Construct with the link-inference cursor seeded to `head`, matching `boot`'s re-seed behavior:
    /// everything written so far is treated as already processed, so a restart does not re-run that
    /// pass. The describer has no cursor — its backlog is derived from the graph's per-memory
    /// described-state, which survives a restart (spec §Write path).
    pub(crate) fn new(head: Seq) -> Self {
        Self {
            link_inference_cursor: Mutex::new(head),
            describe_guard: tokio::sync::Mutex::new(()),
            link_inference_guard: tokio::sync::Mutex::new(()),
            consolidation_cursor: Mutex::new(head),
            consolidation_guard: tokio::sync::Mutex::new(()),
            canonicalize_cursor: Mutex::new(head),
            canonicalize_guard: tokio::sync::Mutex::new(()),
            link_cleanup_cursor: Mutex::new(head),
            link_cleanup_guard: tokio::sync::Mutex::new(()),
        }
    }

    /// Re-seed the link-inference cursor to `head`, treating everything written so far as already
    /// processed. Called at boot after genesis writes, so a restart does not re-infer over old content.
    /// The describer is not re-seeded: its backlog lives in the log-derived per-memory described-state,
    /// so a pre-shutdown describe backlog persists and is picked up rather than silently dropped
    /// (genesis baselines the seeded `self` via `GenesisCompleted`).
    pub(crate) fn reseed(&self, head: Seq) {
        *self.link_inference_cursor.lock() = head;
        *self.consolidation_cursor.lock() = head;
        *self.canonicalize_cursor.lock() = head;
        *self.link_cleanup_cursor.lock() = head;
    }

    /// Catch the vector index up to the log (spec §Storage → vector store). Reads the cursor and the
    /// events past it under brief sync locks, **embeds off every lock**, then applies the embedded
    /// batch under a brief vector-index lock. So the slow `embed().await` holds no guard at all — not
    /// the store, not the graph, not the index — and a concurrent `memory.search` never waits behind a
    /// batch's embedding. A no-op returning `0` on a graph-only instance (no embedder configured).
    pub async fn index_catch_up(&self, engine: &Engine) -> Result<usize, InstanceError> {
        let Some(retrieval) = &engine.retrieval else {
            return Ok(0);
        };
        let from = retrieval
            .vectors
            .lock()
            .cursor()
            .map_err(IndexError::Vector)?
            .next();
        let events = engine
            .store
            .lock()
            .read_from(from)
            .map_err(IndexError::Store)?;
        let count = events.len();
        // Resolve memory names for contextual embeddings before the embed, so the slow
        // `embed().await` holds no graph lock. Only `MemoryContentAppended` events need a
        // name — the contextual embedding's handle prefix. Uses the current name, not the
        // name at append time — intentional, matching the dedup check's name resolution.
        let name_map: std::collections::BTreeMap<MemoryId, MemoryName> = {
            let graph = engine.graph.lock();
            events
                .iter()
                .filter_map(|event| match &event.payload {
                    EventPayload::MemoryContentAppended { id, .. } => graph
                        .memory_by_id(*id)
                        .ok()
                        .flatten()
                        .map(|memory| (*id, memory.name)),
                    _ => None,
                })
                .collect()
        };
        let name_resolver = |id: MemoryId| name_map.get(&id).cloned();
        let batch = embed_batch(retrieval.embedder.as_ref(), &events, Some(&name_resolver)).await?;
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
    pub async fn reembed_if_embedding_model_changed(
        &self,
        engine: &Engine,
    ) -> Result<bool, InstanceError> {
        let Some(retrieval) = &engine.retrieval else {
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
                let now = engine.clock.now();
                engine.store.lock().append(
                    now,
                    EventSource::Orchestration,
                    vec![EventPayload::embedding_model_changed(recorded, configured)],
                )?;
                // Apply the migration into the graph (a no-op there) so graph-head keeps pace with the
                // log, then clear the index and re-embed the whole log under the new model.
                engine
                    .graph
                    .lock()
                    .materialize_from(engine.store.lock().as_ref())?;
                retrieval
                    .vectors
                    .lock()
                    .clear()
                    .map_err(IndexError::Vector)?;
                let indexed = self.index_catch_up(engine).await?;
                tracing::info!(indexed, "re-embed complete; serving resumes");
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    /// Catch synthesized descriptions up to the log: describe every stale memory (description, belief
    /// arbitration, and temporal extraction) — one whose content has changed since the describer last
    /// considered it (spec §Write path → regenerate off the hot path, as a catch-up). The synchronous
    /// counterpart to the background describer — the same dual-mode shape as `index_catch_up` — driven
    /// explicitly by tests so a caller can force regeneration and then read fresh
    /// descriptions. Returns how many memories it considered. The describer guard is held per memory,
    /// not across the whole pass, so a narrow session-open pass interleaves rather than waiting behind
    /// this backlog.
    pub async fn describe_catch_up(
        &self,
        engine: &Engine,
        model: &dyn ModelClient,
    ) -> Result<usize, InstanceError> {
        Ok(run_describe_catch_up(engine, model, &self.describe_guard).await?)
    }

    /// As [`BackgroundPasses::describe_catch_up`], but narrowed to the stale memories among `ids` — the
    /// pass a session open runs over its brief's read set, so it pays only for the descriptions the
    /// brief will read (spec §Starvation bound). A stale memory not in `ids` stays stale for the
    /// background pass.
    pub async fn describe_catch_up_for(
        &self,
        engine: &Engine,
        model: &dyn ModelClient,
        ids: &[MemoryId],
    ) -> Result<usize, InstanceError> {
        Ok(run_describe_catch_up_for(engine, model, &self.describe_guard, ids).await?)
    }

    /// Catch link inference up to the log off the hot path (spec §Write path → link inference):
    /// identify relationships implicit in every memory whose content changed since the cursor,
    /// advancing it. Driven on a timer by the served runtime and explicitly by tests. Returns how many
    /// memories it considered.
    pub async fn link_inference_catch_up(
        &self,
        engine: &Engine,
        model: &dyn ModelClient,
    ) -> Result<usize, InstanceError> {
        let _guard = self.link_inference_guard.lock().await;
        let cursor = *self.link_inference_cursor.lock();
        let (advanced, count) = run_link_inference_catch_up(engine, model, cursor).await?;
        *self.link_inference_cursor.lock() = advanced;
        Ok(count)
    }

    /// Seed the link-inference pass's cursor to log-head, treating every relationship so far as
    /// already inferred. Called at boot and at agent creation, like the describer's, so a restart does
    /// not re-infer over old content.
    pub(crate) fn baseline_link_inference_cursor(
        &self,
        engine: &Engine,
    ) -> Result<(), InstanceError> {
        *self.link_inference_cursor.lock() = engine.store.lock().head()?;
        Ok(())
    }

    /// Catch the consolidation pass up to the log. Returns how many memories it considered. The
    /// activity gate (events since last cursor advance) must be checked by the caller before invoking
    /// this — the background driver does so; the on-demand CLI/console paths always run.
    ///
    /// `start` decides the window: the timer-driven driver resumes from the incremental
    /// [`MaintenanceStart::Cursor`]; the on-demand entry points pass [`MaintenanceStart::FromStart`] to
    /// sweep the whole log, since a fresh instance seeds the cursor to log-head at boot and the
    /// incremental cursor would otherwise make a manual pass a no-op. Either way the advanced cursor is
    /// stored, so a full sweep folds into the timer's cursor too — safe because the pass is idempotent.
    pub async fn consolidation_catch_up(
        &self,
        engine: &Arc<Engine>,
        model: &dyn ModelClient,
        start: MaintenanceStart,
    ) -> Result<usize, InstanceError> {
        let _guard = self.consolidation_guard.lock().await;
        let cursor = start.cursor(&self.consolidation_cursor);
        let (advanced, count) = maintenance::consolidation::catch_up(engine, model, cursor).await?;
        *self.consolidation_cursor.lock() = advanced;
        Ok(count)
    }

    /// Catch the canonicalize pass up to the log. Returns how many stubs it considered. See
    /// [`BackgroundPasses::consolidation_catch_up`] for the `start` (timer vs on-demand) asymmetry.
    pub async fn canonicalize_catch_up(
        &self,
        engine: &Arc<Engine>,
        model: &dyn ModelClient,
        start: MaintenanceStart,
    ) -> Result<usize, InstanceError> {
        let _guard = self.canonicalize_guard.lock().await;
        let cursor = start.cursor(&self.canonicalize_cursor);
        let (advanced, count) = maintenance::canonicalize::catch_up(engine, model, cursor).await?;
        *self.canonicalize_cursor.lock() = advanced;
        Ok(count)
    }

    /// Catch the link-cleanup pass up to the log. Returns how many memories it considered. See
    /// [`BackgroundPasses::consolidation_catch_up`] for the `start` (timer vs on-demand) asymmetry.
    pub async fn link_cleanup_catch_up(
        &self,
        engine: &Arc<Engine>,
        model: &dyn ModelClient,
        start: MaintenanceStart,
    ) -> Result<usize, InstanceError> {
        let _guard = self.link_cleanup_guard.lock().await;
        let cursor = start.cursor(&self.link_cleanup_cursor);
        let (advanced, count) = maintenance::link_cleanup::catch_up(engine, model, cursor).await?;
        *self.link_cleanup_cursor.lock() = advanced;
        Ok(count)
    }

    /// Seed the maintenance pass cursors to log-head, treating everything so far as already
    /// processed. Called at boot and at agent creation, like the link-inference cursor.
    pub(crate) fn baseline_maintenance_cursors(
        &self,
        engine: &Engine,
    ) -> Result<(), InstanceError> {
        let head = engine.store.lock().head()?;
        *self.consolidation_cursor.lock() = head;
        *self.canonicalize_cursor.lock() = head;
        *self.link_cleanup_cursor.lock() = head;
        Ok(())
    }
}

impl Instance {
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
                    match self.passes.index_catch_up(&self.engine).await {
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

    /// The background describer: on each tick, catch synthesized descriptions up to the log off the
    /// turn's hot path (spec §Write path). Idempotent and cursor-resumed, so an idle tick is cheap.
    /// Stops on the shutdown signal; a failure is logged, not fatal — a description stays stale until
    /// the next tick or the forcing guard before a brief.
    ///
    /// Takes the whole arbiter rather than a single handle so it can pick its priority per sweep:
    /// normally it yields to conversation on the background handle, but when its backlog has aged past
    /// the staleness-escape horizon it dispatches the sweep at turn priority instead, so a saturated
    /// instance cannot starve a description that readers see (spec §Write path → freshness before a
    /// brief).
    pub async fn run_describer(
        self: Arc<Self>,
        arbiter: Arc<ModelArbiter>,
        interval: Duration,
        shutdown: impl Future<Output = ()>,
    ) {
        let background = arbiter.background();
        let mut ticker = tokio::time::interval(interval);
        tokio::pin!(shutdown);
        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    // A failed escape evaluation is non-fatal: fall back to yielding rather than
                    // skipping the sweep, so a transient store or graph read never wedges the backlog.
                    let model = match self.describe_should_escalate() {
                        Ok(true) => {
                            observe_describe_priority_escape();
                            arbiter.turn()
                        }
                        Ok(false) => background.clone(),
                        Err(error) => {
                            observe_worker_error("describe");
                            tracing::error!(%error, "describer: could not evaluate the staleness escape; yielding");
                            background.clone()
                        }
                    };
                    match self.passes.describe_catch_up(&self.engine, model.as_ref()).await {
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

    /// Whether the next describe sweep should escalate to conversation priority: its oldest pending
    /// description has aged past the configured escape horizon. The describer is the one background
    /// pass with a user-visible freshness horizon — a read surfaces a stale description — so it alone
    /// escapes the turn-over-background yield; the other passes yield unconditionally. A zero horizon
    /// disables the escape, and an empty backlog never escalates.
    pub(super) fn describe_should_escalate(&self) -> Result<bool, InstanceError> {
        let escape_seconds = Settings::from_store(self.engine.store.lock().as_ref())?
            .concurrency
            .describe_staleness_escape_seconds;
        if escape_seconds <= 0 {
            return Ok(false);
        }
        let Some(oldest) = self.engine.graph.lock().oldest_stale_content_seq()? else {
            return Ok(false);
        };
        let Some(changed_at) = self.engine.store.lock().recorded_at(oldest)? else {
            return Ok(false);
        };
        let age_ms = self
            .engine
            .clock
            .now()
            .as_millisecond()
            .saturating_sub(changed_at.as_millisecond());
        Ok(age_ms >= escape_seconds.saturating_mul(1_000))
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
                    match self.passes.link_inference_catch_up(&self.engine, model.as_ref()).await {
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

    /// The maintenance driver: on each tick, run each maintenance pass if its activity gate fires
    /// (spec §Write path → maintenance passes). Each pass is cursor-resumed and idempotent, so an
    /// idle tick is cheap. Stops on the shutdown signal; a failure is logged, not fatal. Spawned
    /// only when a model is configured; without one there is nothing to run the synthesis calls.
    pub async fn run_maintenance(
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
                    let settings = Settings::from_store(self.engine.store.lock().as_ref())
                        .unwrap_or_default();
                    if !settings.maintenance.enabled {
                        continue;
                    }
                    // Each pass runs if its activity gate fires.
                    if maintenance::activity_gate(
                        &self.engine,
                        *self.passes.consolidation_cursor.lock(),
                        settings.maintenance.consolidation_min_activity,
                    ).unwrap_or(false) {
                        match self.passes.consolidation_catch_up(&self.engine, model.as_ref(), MaintenanceStart::Cursor).await {
                            Ok(considered) if considered > 0 => {
                                tracing::debug!(considered, "consolidation pass consolidated entries")
                            }
                            Ok(_) => {}
                            Err(error) => {
                                observe_worker_error("consolidation");
                                tracing::error!(%error, "consolidation: catch-up failed")
                            }
                        }
                    }
                    if maintenance::activity_gate(
                        &self.engine,
                        *self.passes.canonicalize_cursor.lock(),
                        settings.maintenance.canonicalize_min_activity,
                    ).unwrap_or(false) {
                        match self.passes.canonicalize_catch_up(&self.engine, model.as_ref(), MaintenanceStart::Cursor).await {
                            Ok(considered) if considered > 0 => {
                                tracing::debug!(considered, "canonicalize pass minted profiles")
                            }
                            Ok(_) => {}
                            Err(error) => {
                                observe_worker_error("canonicalize");
                                tracing::error!(%error, "canonicalize: catch-up failed")
                            }
                        }
                    }
                    if maintenance::activity_gate(
                        &self.engine,
                        *self.passes.link_cleanup_cursor.lock(),
                        settings.maintenance.link_cleanup_min_activity,
                    ).unwrap_or(false) {
                        match self.passes.link_cleanup_catch_up(&self.engine, model.as_ref(), MaintenanceStart::Cursor).await {
                            Ok(considered) if considered > 0 => {
                                tracing::debug!(considered, "link cleanup pass retracted redundant entries")
                            }
                            Ok(_) => {}
                            Err(error) => {
                                observe_worker_error("link_cleanup");
                                tracing::error!(%error, "link cleanup: catch-up failed")
                            }
                        }
                    }
                }
                _ = &mut shutdown => break,
            }
        }
        tracing::info!("maintenance driver stopped");
    }
}
