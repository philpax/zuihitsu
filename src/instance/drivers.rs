//! The background timer drivers: the wake-up scheduler, the graph snapshotter, the idle-session
//! sweeper, and the checkpoint sweeper. Each runs on a `tokio::select!` timer loop until a shutdown
//! signal resolves. Unlike the cursor-resumed catch-up workers in [`crate::instance::workers`], these fire
//! globally-due work, checkpoint the graph, consolidate idle sessions, and flush live sessions'
//! working state mid-session.

use std::{
    future::Future,
    sync::{Arc, atomic::AtomicI64},
    time::Duration,
};

use crate::{
    agent::{Flush, TurnView, bounded_buffer_turns, flushed_up_to, run_flush},
    event::{SessionEndCause, TurnRole},
    ids::ConversationId,
    instance::{Instance, InstanceError, OpenSession, SnapshotSchedule},
    memory::scheduler,
    metrics::{observe_flush_turn, observe_wakeups_fired, observe_worker_error},
    model::ModelClient,
    settings::Settings,
    snapshot,
    time::Timestamp,
};

/// What drove a checkpoint sweep. The two triggers apply different gate sets, so
/// [`Instance::checkpoint_live_sessions`] and [`Instance::checkpoint_delta`] branch on it: a timer
/// tick is throttled by all three gates, while a fresh session open needs the parallel state
/// immediately and so waives the cooldown and audience gates.
#[derive(Clone, Copy, Debug)]
pub enum CheckpointTrigger {
    /// The background timer: all three gates (substance, cooldown, and audience) apply.
    Timer,
    /// A fresh session opening for the carried conversation: the substance gate alone applies, the
    /// opener is the audience so audience is waived, and the cooldown is waived — the opening
    /// session's brief needs the parallel conversations' state now. The opener is skipped in the
    /// sweep loop; its own lapsed session is flush-and-ended separately under its lifecycle lock.
    SessionOpen(ConversationId),
}

impl CheckpointTrigger {
    /// Whether the conversation is the session-open trigger's own opener, which the sweep skips —
    /// the opener's lapsed session is flush-and-ended under its lifecycle lock in `ensure_session`,
    /// not swept here. The timer trigger skips nothing.
    fn skips(self, conversation: ConversationId) -> bool {
        matches!(self, CheckpointTrigger::SessionOpen(opener) if opener == conversation)
    }

    /// Whether the cooldown and audience gates apply. The timer sweep throttles on both; a session
    /// open waives them, gating on substance alone.
    fn applies_cooldown_and_audience(self) -> bool {
        matches!(self, CheckpointTrigger::Timer)
    }
}

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
        let compaction = Settings::from_store(self.engine.store.lock().as_ref())?.compaction;
        let idle_gap_ms = compaction.idle_gap_seconds.saturating_mul(1_000);
        let mut closed = 0;
        // Bind the list first so the graph guard drops before the per-session flush `.await` below.
        let open = self.engine.graph.lock().open_sessions()?;
        for (conversation, recovered) in open {
            let live_activity = self
                .sessions
                .get(conversation)
                .map(|open| open.last_activity_millis());
            let last_activity_ms = match live_activity {
                Some(ms) => ms,
                // A recovered session reads only from its own `SessionStarted` seq (an empty carried
                // tail), routed through the bound for a single buffer-read path.
                None => bounded_buffer_turns(
                    self.engine.store.lock().as_ref(),
                    conversation,
                    recovered.start_seq,
                    recovered.start_seq,
                    compaction.carryover_char_budget,
                )?
                .last()
                .map_or(recovered.started_at, |turn| turn.recorded_at)
                .as_millisecond(),
            };
            if now.as_millisecond() - last_activity_ms <= idle_gap_ms {
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
            let stale = match self.sessions.remove_if_matches(conversation, recovered.id) {
                Some(open) => open,
                None => Arc::new(OpenSession {
                    id: recovered.id,
                    vm: self.mint_vm(conversation),
                    brief: recovered.brief,
                    // A stale session reconstructed only to flush-and-close runs no turn, so its brief
                    // read set is never consulted.
                    brief_memories: Vec::new(),
                    started_at: recovered.started_at,
                    last_activity: AtomicI64::new(last_activity_ms),
                    start_seq: recovered.start_seq,
                    session_start_seq: recovered.start_seq,
                }),
            };
            self.flush_and_end(conversation, stale.as_ref(), model, SessionEndCause::Idle)
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

    /// Checkpoint-flush every live session whose unflushed working state is due to reach memory
    /// mid-session (spec §Compaction → checkpoint flush) — the cross-conversation sync a session's
    /// end-flush alone cannot give: without it, conversation B learns nothing of a parallel
    /// conversation A until A goes idle or compacts. Two triggers drive it, distinguished by
    /// `trigger`:
    ///
    /// - [`CheckpointTrigger::Timer`]: the background sweeper's tick. Each candidate is gated three
    ///   ways ([`Instance::checkpoint_delta`]) — substance, cooldown, and audience.
    /// - [`CheckpointTrigger::SessionOpen`]: a fresh session opening for the opener conversation, so
    ///   the parallel conversations' state reaches memory before the opener's brief composes. The
    ///   opener is skipped entirely (its own lapsed session is flush-and-ended separately under its
    ///   lifecycle lock in `ensure_session`), and the substance gate alone applies — cooldown and
    ///   audience are waived, since the opener is the audience and is not yet in the live map for the
    ///   audience check to see.
    ///
    /// A conversation whose turn is currently in flight is skipped — checked both in the cheap
    /// pre-check and again under the lifecycle lock, since the race can open between those two points.
    /// A turn past `ensure_session` does not hold the lifecycle lock while it generates, so without
    /// this the sweep could snapshot that conversation's buffer mid-turn — an inbound message with its
    /// reply not yet committed — and flush an unanswered question into memory. Deferring the candidate
    /// loses nothing: the substance gate finds the same delta on the next tick once the turn exits.
    ///
    /// An eligible candidate is flushed under its conversation's lifecycle lock, with the gates
    /// re-validated there, so a message arriving mid-flush waits in `ensure_session` (on its own
    /// conversation's flush only — the flush is delta-sized) and the idle sweep's close of the same
    /// session never interleaves. The flush turn is ordinary and the session stays open: no
    /// `SessionEnded`, no carryover, no brief rebuild — the turn simply rides the live buffer, and its
    /// seq becomes the next watermark. Both triggers respect `settings.checkpoint.enabled` as the
    /// master switch. Returns how many sessions flushed. Driven on a timer by
    /// [`Instance::run_checkpoint_sweeper`]; also driven at a session open by `ensure_session`, and
    /// callable directly to sweep once on demand.
    pub async fn checkpoint_live_sessions(
        &self,
        model: &dyn ModelClient,
        trigger: CheckpointTrigger,
    ) -> Result<usize, InstanceError> {
        let now = self.engine.clock.now();
        let settings = Settings::from_store(self.engine.store.lock().as_ref())?;
        if !settings.checkpoint.enabled {
            return Ok(0);
        }
        let mut flushed = 0;
        for (conversation, open) in self.sessions.live() {
            // The opener drives its own lapsed session's close through `ensure_session`; a
            // session-open sweep must not touch it here.
            if trigger.skips(conversation) {
                continue;
            }
            // Skip a conversation whose turn is in flight: it does not hold the lifecycle lock while it
            // generates, so flushing now could snapshot its buffer mid-turn (an inbound with no reply).
            // The delta is not lost — the next tick's substance gate finds it once the turn exits.
            if self.turns.has_admitted_turn(conversation) {
                continue;
            }
            // A cheap pre-check without the lock, so the lifecycle lock is taken only for a real
            // candidate rather than serializing every live conversation each tick.
            if self
                .checkpoint_delta(conversation, &open, &settings, now, trigger)?
                .is_none()
            {
                continue;
            }
            let lifecycle = self.lifecycle_lock(conversation);
            let _lifecycle = lifecycle.lock().await;
            // Re-validate under the lock: a message that arrived since the candidate list was
            // captured may have closed this session (an idle reopen or a compaction), and a delta
            // another closer already flushed must not flush twice.
            if self
                .sessions
                .get(conversation)
                .is_none_or(|current| current.id != open.id)
            {
                continue;
            }
            // Re-check the in-flight guard under the lock: a turn can have admitted between the
            // pre-check above and here (the lifecycle lock and the turn slot are distinct locks).
            if self.turns.has_admitted_turn(conversation) {
                continue;
            }
            let Some(delta) =
                self.checkpoint_delta(conversation, &open, &settings, now, trigger)?
            else {
                continue;
            };
            // An ordinary flush turn over the unflushed delta only — the frozen brief still frames
            // it, so repeat checkpoints never re-flush the same turns. Its writes are the agent's
            // own; its terminal is silent (empty agent text), so nothing is delivered to
            // participants and the next turn's buffer replays only its Lua steps.
            let present_set = self.engine.graph.lock().session_participants(open.id)?;
            run_flush(Flush {
                session: &open.vm,
                model,
                engine: self.engine.clone(),
                brief: &open.brief,
                session_started_at: open.started_at,
                buffer: &delta.buffer[delta.start..],
                present_set: &present_set,
                max_steps: settings.turn.max_steps as usize,
                block_timeout: Duration::from_secs(
                    settings.turn.block_timeout_seconds.max(0) as u64
                ),
                max_block_attempts: settings.turn.max_block_attempts.max(1) as u32,
                max_entry_chars: settings.memory.max_entry_chars.max(1) as usize,
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
            tracing::info!(?conversation, session = ?open.id, "checkpoint-flushed a live session");
            flushed += 1;
        }
        Ok(flushed)
    }

    /// The background checkpoint-sweep driver: on each tick, checkpoint-flush every live session
    /// whose gates pass, so a parallel conversation can read this one's working state before it goes
    /// idle. Long-lived; a sweep failure is logged, never propagated.
    pub async fn run_checkpoint_sweeper(
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
                    match self.checkpoint_live_sessions(model.as_ref(), CheckpointTrigger::Timer).await {
                        Ok(flushed) if flushed > 0 => {
                            tracing::info!(flushed, "checkpoint sweep flushed live sessions")
                        }
                        Ok(_) => {}
                        Err(error) => {
                            observe_worker_error("checkpoint");
                            tracing::error!(%error, "checkpoint sweep failed")
                        }
                    }
                }
                _ = &mut shutdown => break,
            }
        }
        tracing::info!("checkpoint sweep driver stopped");
    }

    /// Evaluate the checkpoint gates over one live session, returning its unflushed delta when the
    /// gates that `trigger` applies pass and `None` when any blocks. The gates, each keyed to the
    /// session's flush watermark (the last flush turn in its buffer, or its start — [`flushed_up_to`],
    /// derived from the log so replay reproduces it):
    ///
    /// 1. *Substance*: the delta past the watermark carries at least `min_delta_chars` of
    ///    participant and agent turn text — there is working state worth a model call. Both triggers
    ///    apply it.
    /// 2. *Cooldown*: at least `cooldown_seconds` have passed since the watermark turn was recorded
    ///    (or since the session started, if it has never flushed) — checkpoints never thrash. Applied
    ///    by [`CheckpointTrigger::Timer`], waived by [`CheckpointTrigger::SessionOpen`] (the opening
    ///    session's brief needs the parallel state now, not after a cooldown).
    /// 3. *Audience*: some other conversation has a live session active at or since the watermark —
    ///    with a single live conversation, the only reader of the flushed tail is the conversation
    ///    itself, which already has it in the buffer. Applied by [`CheckpointTrigger::Timer`], waived
    ///    by [`CheckpointTrigger::SessionOpen`] (the opener *is* the audience, and it is not yet in
    ///    the live map for this check to see).
    fn checkpoint_delta(
        &self,
        conversation: ConversationId,
        open: &OpenSession,
        settings: &Settings,
        now: Timestamp,
        trigger: CheckpointTrigger,
    ) -> Result<Option<UnflushedDelta>, InstanceError> {
        let checkpoint = &settings.checkpoint;
        let buffer = bounded_buffer_turns(
            self.engine.store.lock().as_ref(),
            conversation,
            open.start_seq,
            open.session_start_seq,
            settings.compaction.carryover_char_budget,
        )?;
        let watermark = flushed_up_to(&buffer, open.session_start_seq);
        // The watermark's wall-clock anchor: when the last flush turn was recorded, or the session's
        // open when it has never flushed — what the cooldown and audience gates measure against.
        let anchor = buffer
            .iter()
            .find(|turn| turn.seq == watermark)
            .map_or(open.started_at, |turn| turn.recorded_at);
        let start = buffer.partition_point(|turn| turn.seq <= watermark);

        let delta_chars: usize = buffer[start..]
            .iter()
            .filter(|turn| matches!(turn.role, TurnRole::Participant | TurnRole::Agent))
            .map(|turn| turn.text.chars().count())
            .sum();
        if (delta_chars as i64) < checkpoint.min_delta_chars {
            return Ok(None);
        }

        // The cooldown and audience gates apply to the timer sweep only; a session open waives both
        // (see [`CheckpointTrigger`]).
        if trigger.applies_cooldown_and_audience() {
            if now.as_millisecond() - anchor.as_millisecond()
                < checkpoint.cooldown_seconds.saturating_mul(1_000)
            {
                return Ok(None);
            }

            // Activity "since the watermark" is inclusive: a tie (coarse clocks stamp a burst at one
            // instant) errs toward flushing, the safe direction.
            let audience = self.sessions.live().iter().any(|(other, session)| {
                *other != conversation && session.last_activity_millis() >= anchor.as_millisecond()
            });
            if !audience {
                return Ok(None);
            }
        }

        Ok(Some(UnflushedDelta { buffer, start }))
    }
}

/// A live session's buffer with the index where its unflushed delta begins — the turns past the flush
/// watermark, the slice a checkpoint flush scopes its prompt to.
struct UnflushedDelta {
    buffer: Vec<TurnView>,
    start: usize,
}
