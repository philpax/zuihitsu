//! The background timer drivers: the wake-up scheduler, the graph snapshotter, and the idle-session
//! sweeper. Each runs on a `tokio::select!` timer loop until a shutdown signal resolves. Unlike the
//! cursor-resumed catch-up workers in [`super::workers`], these fire globally-due work, checkpoint
//! the graph, and consolidate idle sessions.

use std::{
    future::Future,
    sync::{Arc, atomic::AtomicI64},
    time::Duration,
};

use crate::{
    agent::buffer_turns,
    memory::scheduler,
    metrics::{observe_wakeups_fired, observe_worker_error},
    model::ModelClient,
    settings::Settings,
    snapshot,
    time::Timestamp,
};

use super::{Instance, InstanceError, OpenSession, SnapshotSchedule};

impl Instance {
    /// Fire every globally-due wake-up as of `now` and reconcile the graph if any fired (spec §Scheduled
    /// work). Shared by the session-open catch-up and the background driver, so both fire with identical
    /// semantics — it is global (every due trigger, not one conversation's) and idempotent (a fired
    /// trigger is no longer due). Holds the graph guard before the store, per the lock-ordering rule.
    pub(super) fn fire_due_now(&self, now: Timestamp) -> Result<usize, InstanceError> {
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
}
