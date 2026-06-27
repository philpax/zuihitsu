//! The background catch-up workers: the indexer, describer, adjudicator, and link-inference pass.
//! Each has a synchronous `catch_up` (driven explicitly by tests and the eval harness) and a
//! background `run_*` loop (driven on a timer by the served runtime). All are cursor-resumed and
//! idempotent, so an idle tick is cheap.

use std::{future::Future, sync::Arc, time::Duration};

use crate::{
    agent::{run_adjudicate_catch_up, run_describe_catch_up, run_link_inference_catch_up},
    event::EventPayload,
    metrics::observe_worker_error,
    model::{
        ModelClient,
        index::{IndexError, apply_batch, embed_batch},
    },
};

use super::Instance;
use crate::InstanceError;

impl Instance {
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
}
