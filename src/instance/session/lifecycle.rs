//! Session lifecycle: opening, closing, flushing, and resolving the operator.

use std::{
    sync::{Arc, atomic::AtomicI64},
    time::Duration,
};

use crate::{
    InstanceFeatures,
    agent::{Flush, bounded_buffer_turns, lua::Session, run_flush},
    event::{ConversationRef, EventPayload, Initiation, TurnRole},
    ids::{ConversationId, MemoryId, MemoryName, NamespacedMemoryName, SessionId, TurnId},
    memory::{brief, scheduler},
    metrics::{
        observe_flush_turn, observe_session_closed, observe_session_opened,
        observe_wakeups_surfaced,
    },
    model::ModelClient,
    settings::Settings,
};

use super::super::{Instance, InstanceError, OpenSession};

impl Instance {
    /// The features this instance enables — the gate the Lua registration, the API reference, and the
    /// scaffold all read, so the runtime surface, the prompt's description, and the baked guidance
    /// stay in lockstep.
    pub fn features(&self) -> InstanceFeatures {
        self.features
    }

    /// A fresh session VM for a conversation, carrying the MCP projection when servers are connected.
    pub(crate) fn mint_vm(&self, conversation: ConversationId) -> Session {
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
    pub(crate) async fn flush_and_end(
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
        let buffer = bounded_buffer_turns(
            self.engine.store.lock().as_ref(),
            conversation,
            open.start_seq,
            open.session_start_seq,
            settings.compaction.carryover_char_budget,
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
    pub(crate) async fn ensure_session(
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
            // A recovered session reads only from its own `SessionStarted` seq (the carried tail is not
            // reconstructable from the log alone), so the read start and this session's start coincide
            // — an empty tail, but routed through the bound for a single buffer-read path.
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
        let old = self.sessions.remove(conversation);
        if let Some(old) = old {
            self.flush_and_end(conversation, old.as_ref(), model)
                .await?;
        }

        // A pending carryover from a just-compacted session seeds the new one: the next buffer read
        // starts at the carried tail (not this `SessionStarted`), the boundary is recorded as
        // `seeded_from_turn` for faithful replay, and the touch-derived working set augments the new
        // brief as active threads (spec §Compaction → carryover).
        let carryover = self.sessions.take_carryover(conversation);
        let seeded_from_turn = carryover.as_ref().map(|carry| ConversationRef {
            conversation,
            turn: Some(carry.seeded_from_turn),
        });
        let working_set: &[MemoryId] = carryover
            .as_ref()
            .map_or(&[], |carry| carry.working_set.as_slice());

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
            ids.extend_from_slice(working_set);
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
    pub(crate) fn resolve_or_mint_operator(&self) -> Result<MemoryId, InstanceError> {
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
