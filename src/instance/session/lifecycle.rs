//! Session lifecycle: opening, closing, flushing, and resolving the operator.

use std::{
    sync::{Arc, atomic::AtomicI64},
    time::Duration,
};

use crate::{
    InstanceFeatures,
    agent::{
        Flush, bounded_buffer_turns, lua::Session, recent_touched, run_flush, session_touched,
    },
    event::{ConversationRef, EventPayload, EventSource, Initiation, SessionEndCause, TurnRole},
    ids::{ConversationId, MemoryId, MemoryName, NamespacedMemoryName, Seq, SessionId, TurnId},
    memory::{brief, scheduler},
    metrics::{
        observe_flush_turn, observe_session_closed, observe_session_opened,
        observe_wakeups_surfaced,
    },
    model::ModelClient,
    settings::Settings,
    time::{self, Timestamp},
};

use crate::instance::{
    CheckpointTrigger, Instance, InstanceError, OpenSession, TailSeed, carryover_tail,
};

/// The previous session's reconstructed tail: its char-budget extent (seeding the new buffer) plus the
/// time of its last turn (deciding warm vs. cold). Both come from the log — nothing is cached across
/// the close (issue #86).
struct PreviousTail {
    seed: TailSeed,
    last_activity: Timestamp,
}

impl Instance {
    /// The features this instance enables — the gate the Lua registration, the API reference, and the
    /// scaffold all read, so the runtime surface, the prompt's description, and the baked guidance
    /// stay in lockstep.
    pub fn features(&self) -> InstanceFeatures {
        self.features
    }

    /// A fresh session VM for a conversation, carrying the MCP projection when servers are connected
    /// and the `web.markdown` projection when a fetcher is connected.
    pub(crate) fn mint_vm(&self, conversation: ConversationId) -> Session {
        let base = match &self.mcp {
            Some(runtime) => Session::with_mcp(
                conversation,
                runtime.host.clone(),
                runtime.catalogue.clone(),
                self.features,
            ),
            None => Session::new(conversation, self.features),
        };
        base.with_web(self.web.clone())
    }

    /// Flush a closing session's working state to memory, then record `SessionEnded` with its `cause`.
    /// The budget-gated pre-compaction flush gives a substantive session (at least `flush_min_turns`
    /// of its own turns) one turn to write durable memory before the cut, so nothing it learned is lost
    /// between its last write and the next conversation; a light session skips it, so the hot-path model
    /// call is paid only when there is state worth saving. The flush runs **before** `SessionEnded`, so
    /// a flush failure leaves the session standing for a retry rather than dropping its state. Shared by
    /// every close — the budget-compaction cut ([`SessionEndCause::Compaction`]), the idle sweep and
    /// lapsed-live closes ([`SessionEndCause::Idle`]), and the cold-start recovery close
    /// ([`SessionEndCause::Recovery`]) — which is why the cause is a parameter. Nothing is staged for the
    /// next session: the reopen reconstructs the tail from the log (issue #86). The caller has already
    /// removed `open` from the sessions map. Returns whether the flush ran.
    pub(crate) async fn flush_and_end(
        &self,
        conversation: ConversationId,
        open: &OpenSession,
        model: &dyn ModelClient,
        cause: SessionEndCause,
    ) -> Result<bool, InstanceError> {
        // The caller holds this conversation's lifecycle lock (see [`Instance::lifecycle`]), so the
        // open-check is reliable here — no other path can be closing this session concurrently. Skip if
        // it is already ended: a path that held the lock before us (the sweep, or the recovery close) has
        // closed it.
        if !self.engine.graph.lock().session_is_open(open.id)? {
            return Ok(false);
        }
        let settings = Settings::from_store(self.engine.store.lock().as_ref())?;
        let buffer = bounded_buffer_turns(
            self.engine.store.lock().as_ref(),
            conversation,
            open.start_seq,
            open.session_start_seq,
            settings.compaction.carryover_char_budget,
        )?;
        // Gate the flush on this session's *own* turns, not the carried tail: a tail seeds the buffer
        // for the flush's context, but it is a prior session's substance (already consolidated when that
        // session closed), so counting it would flush a session that reopened after an idle gap and said
        // almost nothing. The buffer is in seq order with the trimmed tail below `session_start_seq`, so
        // the own turns are the suffix at or after it.
        let own_turns = buffer
            .iter()
            .filter(|turn| turn.seq >= open.session_start_seq)
            .count();
        let flushed = own_turns as i64 >= settings.compaction.flush_min_turns;
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
        }
        open.vm.shutdown_mcp().await;
        let now = self.engine.clock.now();
        self.engine.store.lock().append(
            now,
            EventSource::Orchestration,
            vec![EventPayload::session_ended(conversation, open.id, cause)],
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

    /// The previous session's raw-transcript tail, reconstructed from the log — its char-budget extent
    /// plus the time of its last turn (issue #86). `None` when there is no previous session or it left
    /// no turns. Reads the previous session's *own* turns from its `SessionStarted` seq, so the tail
    /// advances into each session with the seam rather than re-spanning every session since the original
    /// cut, and it includes any flush turn that close appended, so the next session's flush watermark
    /// rides across the seam.
    fn previous_session_tail(
        &self,
        conversation: ConversationId,
        previous_start: Option<Seq>,
        char_budget: i64,
    ) -> Result<Option<PreviousTail>, InstanceError> {
        let Some(previous_start) = previous_start else {
            return Ok(None);
        };
        let own = bounded_buffer_turns(
            self.engine.store.lock().as_ref(),
            conversation,
            previous_start,
            previous_start,
            char_budget,
        )?;
        Ok(carryover_tail(&own, char_budget).map(|seed| PreviousTail {
            seed,
            last_activity: own
                .last()
                .expect("carryover_tail is Some only for a non-empty buffer")
                .recorded_at,
        }))
    }

    /// Ensure a live session for `conversation`. Reuse the open one if activity is within the idle gap.
    /// Otherwise, on a cold start (no live session in this process), recover a session still open in the
    /// log — left by a restart or a passive graceful exit: within the idle gap resume it untouched (an
    /// identical prompt prefix, so the serving cache survives the restart), past it close-with-flush.
    /// Then, for a stale live session or after a recovered close, open a fresh one — composing and
    /// freezing its brief and minting a fresh VM. Boundaries are recorded (`SessionStarted` /
    /// `SessionEnded`), never recomputed at replay.
    pub(crate) async fn ensure_session(
        &self,
        conversation: ConversationId,
        present_set: &[MemoryId],
        speakers: &[MemoryId],
        model: &dyn ModelClient,
    ) -> Result<Arc<OpenSession>, InstanceError> {
        // Before taking this conversation's lifecycle lock, checkpoint-flush the *other* live
        // conversations if a fresh session is about to open for this one — so their working state
        // reaches memory before this session's brief composes (the brief then reads the just-flushed
        // state) and before its first turn dispatches, rather than the two racing at the shared model.
        //
        // The ordering is load-bearing: the sweep takes each other conversation's lifecycle lock
        // internally, and no code path may ever hold two lifecycle locks at once (that is the deadlock
        // guard), so this must run before this conversation's lock is acquired below.
        //
        // A cheap unlocked pre-check keeps the hot reuse path free: if this conversation has a live
        // session within the idle gap, the call below will reuse it — no fresh session, so nothing to
        // sweep for. The pre-check is unlocked, so a concurrent open may have inserted a live session
        // by the time the lock is taken; the sweep was then unnecessary but harmless — the substance
        // gate self-limits, and each candidate is re-validated under its own lifecycle lock. The
        // `SessionOpen` trigger skips this conversation's own lapsed session, which is flush-and-ended
        // separately under the lock below.
        {
            let settings = Settings::from_store(self.engine.store.lock().as_ref())?;
            let now = self.engine.clock.now();
            let idle_gap_ms = settings.compaction.idle_gap_seconds.saturating_mul(1_000);
            let will_reuse = self
                .sessions
                .get(conversation)
                .is_some_and(|open| now.as_millis() - open.last_activity_millis() <= idle_gap_ms);
            if settings.checkpoint.flush_on_open && !will_reuse {
                match self
                    .checkpoint_live_sessions(model, CheckpointTrigger::SessionOpen(conversation))
                    .await
                {
                    Ok(flushed) if flushed > 0 => {
                        tracing::info!(
                            flushed,
                            ?conversation,
                            "session-open checkpoint flushed live sessions"
                        )
                    }
                    Ok(_) => {}
                    // Another conversation's flush failure must not fail this conversation's turn — log
                    // it and continue; the background timer sweep retries it.
                    Err(error) => {
                        tracing::warn!(
                            %error,
                            ?conversation,
                            "session-open checkpoint flush failed; proceeding with the new session"
                        )
                    }
                }
            }
        }

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
            match self.sessions.get(conversation) {
                Some(open) if now.as_millis() - open.last_activity_millis() <= idle_gap_ms => {
                    open.touch(now);
                    return Ok(open);
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
            // A recovered session reads from its own `SessionStarted` seq — its own tail is
            // reconstructed below at the fresh open, from the log, once it has been closed.
            let buffer = bounded_buffer_turns(
                self.engine.store.lock().as_ref(),
                conversation,
                recovered.start_seq,
                recovered.start_seq,
                settings.compaction.carryover_char_budget,
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
                // A recovered session's brief read set is not reconstructed here; the ambient pass then
                // simply does not dedup against the brief for the brief span of a resumed cold-start
                // session, at worst repeating a memory the brief already names.
                brief_memories: Vec::new(),
                started_at: recovered.started_at,
                last_activity: AtomicI64::new(last_activity.as_millis()),
                start_seq: recovered.start_seq,
                session_start_seq: recovered.start_seq,
            };
            if resumable {
                open.touch(now);
                let open = Arc::new(open);
                self.sessions.insert(conversation, open.clone());
                tracing::info!(?conversation, session = ?open.id, "resumed an open session after a cold start");
                return Ok(open);
            }
            self.flush_and_end(conversation, &open, model, SessionEndCause::Recovery)
                .await?;
            tracing::info!(?conversation, session = ?open.id, "flushed and closed a stale recovered session");
        }

        // Catch the wake-up scheduler up to now before the session opens, so a just-due item can
        // surface in this session if it is eligible (the drain below reads the fired surface). The
        // background driver ([`Instance::run_scheduler`]) fires continuously on a timer; this catch-up
        // stays for immediacy at session open and is idempotent with it.
        self.fire_due_now(now)?;

        // A lapsed live session ends before the new one opens: take it out under the map guard (so no
        // guard is held across the flush's `.await`), then flush-and-end it as an idle close — it went
        // quiet past the gap, which is why control reached here rather than the reuse path above.
        let old = self.sessions.remove(conversation);
        if let Some(old) = old {
            self.flush_and_end(conversation, old.as_ref(), model, SessionEndCause::Idle)
                .await?;
        }

        // Reconstruct the carryover from the log, not from any cached runtime state (issue #86). The
        // previous session's own turns are all in the event log, so at reopen its char-budget tail is
        // re-derivable: the new buffer reads from that tail (recorded as `seeded_from_turn` for faithful
        // replay) rather than from this `SessionStarted`. This survives a restart between the close and
        // the reopen — the exact case an in-memory stash dropped — and needs the previous session's
        // `SessionStarted` seq, which the graph holds whether that session is still open or already
        // closed (the lapsed and recovered closes above have just ended it, so it is the latest).
        let previous_start = self.engine.graph.lock().last_session_start(conversation)?;
        let tail = self.previous_session_tail(
            conversation,
            previous_start,
            settings.compaction.carryover_char_budget,
        )?;
        let seeded_from_turn = tail.as_ref().map(|tail| ConversationRef {
            conversation,
            turn: Some(tail.seed.seeded_from_turn),
        });

        // The active threads the new session re-surfaces, chosen by the reopen *gap* measured from the
        // previous session's last turn. A reopen within the idle gap reads as a *warm* continuation —
        // near-always a promptly-reopened compaction, since an idle or recovery close only fires after
        // the gap of silence — so it carries the previous session's own touched set (precise,
        // this-conversation-scoped). At or past the gap it is a *cold* resumption (an idle timeout, a
        // recovery, or a compaction nobody answered for a while), so it derives the threads from recent
        // cross-conversation activity instead, so a fresh session re-surfaces what a warm continuation
        // would rather than opening blank (issue #35). The warm/idle boundary is approximate at the
        // edge: the idle sweep measures silence from the message-arrival `touch`, while this measures
        // from the last turn's `recorded_at` (stamped after the model call), so a live session swept
        // idle and reopened within one model-latency of the gap can read as warm. Benign — it only
        // picks the working-set *source*, both re-filtered through the same visibility predicate against
        // the new present set, and the raw tail is carried either way (spec §Compaction → working-set
        // carryover). The recorded `SessionEnded.cause` is provenance; this decision reads the gap.
        let warm = tail
            .as_ref()
            .is_some_and(|tail| now.as_millis() - tail.last_activity.as_millis() < idle_gap_ms);
        let working_set: Vec<MemoryId> = if warm {
            let previous_start = previous_start.expect("a tail implies a previous session");
            session_touched(
                self.engine.store.lock().as_ref(),
                conversation,
                previous_start,
            )?
        } else {
            let window_days = settings.brief.cold_open_window_days.max(0);
            let limit = settings.brief.cold_open_threads.max(0) as usize;
            if window_days == 0 || limit == 0 {
                Vec::new()
            } else {
                let since = Timestamp::from_millis(
                    now.as_millis()
                        .saturating_sub(window_days * time::MILLIS_PER_DAY),
                );
                recent_touched(self.engine.store.lock().as_ref(), since, limit)?
            }
        };

        // Force the description catch-up before composing the brief, so it never reads stale prose for
        // memories a prior turn or the pre-compaction flush just wrote (spec §Starvation bound →
        // composing a brief forces the catch-up). Narrowed to the brief's own read set — the present
        // set, the room's context memory, the carried working set, and the agent's `self` — so a
        // session open pays only for the descriptions its brief reads, not a whole concurrent backlog
        // turn A left; the rest stays stale for the background pass. No lock is held across the model
        // call.
        let context = self
            .engine
            .graph
            .lock()
            .context_for_conversation(conversation)?;
        let brief_memories = {
            let graph = self.engine.graph.lock();
            let mut ids = present_set.to_vec();
            ids.extend_from_slice(&working_set);
            ids.extend(context);
            ids.extend(graph.self_memory()?.map(|memory| memory.id));
            ids
        };
        self.describe_catch_up_for(model, &brief_memories).await?;
        self.engine
            .graph
            .lock()
            .materialize_from(self.engine.store.lock().as_ref())?;
        let brief = brief::compose(
            &self.engine.graph.lock(),
            &settings.brief,
            &brief::BriefRequest {
                present_set,
                speakers,
                current_context: context,
                working_set: &working_set,
                now,
            },
        )?;
        let id = SessionId::generate();
        let committed = self.engine.store.lock().append(
            now,
            EventSource::Orchestration,
            vec![EventPayload::SessionStarted {
                conversation,
                id,
                participants: present_set.to_vec(),
                started_at: now,
                seeded_from_turn,
                brief: brief.clone(),
                working_set,
                initiators: speakers.to_vec(),
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
            brief_memories,
            started_at: now,
            last_activity: AtomicI64::new(now.as_millis()),
            start_seq: tail
                .map(|tail| tail.seed.from_seq)
                .unwrap_or(session_start_seq),
            session_start_seq,
        });
        self.sessions.insert(conversation, open.clone());

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
                brief: None,
            }];
            for (entry_id, memory) in drained.entries {
                payloads.push(EventPayload::scheduled_item_surfaced(
                    entry_id, memory, id, now,
                ));
            }
            self.engine
                .store
                .lock()
                .append(now, EventSource::Orchestration, payloads)?;
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
    pub(crate) fn resolve_or_mint_operator(&self) -> Result<MemoryId, InstanceError> {
        let operator = MemoryName::from(NamespacedMemoryName::operator());
        if let Some(memory) = self.engine.graph.lock().memory_by_name(&operator)? {
            return Ok(memory.id);
        }
        let id = MemoryId::generate();
        let now = self.engine.clock.now();
        self.engine.store.lock().append(
            now,
            EventSource::Orchestration,
            vec![EventPayload::memory_created(id, operator)],
        )?;
        self.engine
            .graph
            .lock()
            .materialize_from(self.engine.store.lock().as_ref())?;
        Ok(id)
    }

    /// Tear down the live sessions at server shutdown: drain the session map and shut each session's
    /// MCP instances down (best-effort). Called by the serving host once the HTTP server has stopped
    /// accepting. Dropping the drained sessions also releases their VMs.
    pub async fn shutdown(&self) {
        let sessions: Vec<Arc<OpenSession>> = self.sessions.drain();
        for session in &sessions {
            session.vm.shutdown_mcp().await;
        }
        drop(sessions);
    }
}
