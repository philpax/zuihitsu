//! The session machinery shared by both facets: opening/continuing a session and running one turn,
//! plus the supporting runtime types (the routed-turn bundle, the compaction carryover, and the live
//! open-session backing a conversation). On [`super::Instance`] (not a facet) so the platform
//! `route_message` and the operator `imprint` both reach it.

use std::{
    sync::{
        Arc,
        atomic::{AtomicI64, Ordering},
    },
    time::Duration,
};

use tracing::Instrument;

use crate::{
    InstanceFeatures,
    agent::{
        Flush, Turn, TurnError, TurnOutcome, TurnReport, TurnView, bounded_buffer_turns,
        lua::Session, run_flush, run_turn,
    },
    event::{EventPayload, Initiation, PromptTemplateName, TurnRole},
    ids::{ConversationId, MemoryId, MemoryName, NamespacedMemoryName, Seq, SessionId, TurnId},
    memory::{brief, memory_block::Authority, scheduler},
    metrics::{
        observe_flush_turn, observe_session_closed, observe_session_opened, observe_turn,
        observe_turn_error, observe_wakeups_surfaced,
    },
    model::ModelClient,
    settings::Settings,
    time::Timestamp,
};

use super::{Instance, InstanceError};

/// The raw-transcript carryover a compaction stages for the next session (spec §Compaction →
/// raw-transcript carryover). The oldest carried turn is both the `seeded_from_turn` boundary
/// recorded on the new `SessionStarted` and the `from_seq` the new session's buffer is read from, so
/// the carried tail plus the new turns reconstruct the post-cut buffer.
pub(crate) struct Carryover {
    pub seeded_from_turn: TurnId,
    pub from_seq: Seq,
    /// The memories the ending session touched (read or wrote), re-surfaced in the new session's
    /// brief as active threads — the touch-derived working set (spec §Compaction → working-set
    /// carryover).
    pub working_set: Vec<MemoryId>,
}

/// The live session backing a conversation (runtime state, see [`super::Instance::sessions`]). Held
/// behind an `Arc` in the `sessions` map, so a running turn keeps its session alive without the map
/// guard; only `last_activity` is mutated after open, so it is an atomic the reuse path bumps through
/// `&self`.
pub(crate) struct OpenSession {
    pub id: SessionId,
    pub vm: Session,
    pub brief: String,
    /// When the session opened — the time frozen into the system prompt's "the session begins on …",
    /// so every turn in the session sends an identical system prefix (the live wall clock rides in the
    /// per-message stamps instead). Holding it stable is what lets the serving layer reuse the prefix
    /// cache across the session's turns.
    pub started_at: Timestamp,
    /// The last-activity wall-clock in epoch millis, the idle-gap is measured from. Atomic so the
    /// idle-reuse path can bump it through the shared `&OpenSession` without a map-wide write lock.
    pub last_activity: AtomicI64,
    /// The log seq the live buffer is read from: the `SessionStarted` seq for a fresh or idle-opened
    /// session, or a carried tail's seq across a compaction seam (so the carryover plus this
    /// session's turns reconstruct the buffer — see [`buffer_turns`]).
    pub start_seq: Seq,
    /// This session's own `SessionStarted` seq — where its own turns begin, at or after `start_seq`.
    /// It splits the buffer read at turn time (and at the flush): the carried tail below it is
    /// re-trimmed to the carryover char budget, while this session's own turns ride whole, so the
    /// buffer stays bounded across compaction seams (see [`bounded_buffer_turns`]). Equal to
    /// `start_seq` for a fresh or idle-opened session (an empty tail).
    pub session_start_seq: Seq,
}

impl OpenSession {
    /// The last-activity time in epoch millis.
    pub fn last_activity_millis(&self) -> i64 {
        self.last_activity.load(Ordering::Relaxed)
    }

    /// Record `now` as the last activity (the idle-reuse bump).
    pub fn touch(&self, now: Timestamp) {
        self.last_activity.store(now.as_millis(), Ordering::Relaxed);
    }
}

/// One routed turn — the inbound message and its routing context, bundled so
/// [`super::Instance::run_session_turn`] takes the routed turn as a whole. Shared by the platform
/// `route_message` and the operator `imprint` paths.
pub(super) struct RoutedTurn<'a> {
    pub conversation: ConversationId,
    pub present_set: &'a [MemoryId],
    pub participant: MemoryId,
    pub inbound: &'a str,
    pub template: PromptTemplateName,
    pub authority: Authority,
}

impl Instance {
    /// Open or continue the session for `conversation`, then run one turn of `inbound` from
    /// `participant` under `template`/`authority`, returning its report and the live buffer it saw
    /// (the buffer the caller's compaction trigger measures). The shared core behind
    /// `Platform::route_message` and `Control::imprint`.
    pub(super) async fn run_session_turn(
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
                    TurnOutcome::Deferred => "deferred",
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
        // Presence sync: anyone on this message who is not yet among the live session's participants
        // arrived mid-session — most clients only ever deliver messages, so the message itself is the
        // join signal, not a `/platform/join` post. Each newcomer gets the same treatment as the
        // explicit endpoint — a `ParticipantJoined` plus an injected join-brief — appended here,
        // before `run_turn` appends the inbound turn, so the brief precedes the message in the
        // buffer. A fresh session is a natural no-op: its `SessionStarted.participants` already carry
        // the full present set. Departures deliberately have no event: the per-turn visibility
        // predicate evaluates against the message's own present set (`routed.present_set` flows into
        // `run_turn`), so a departed participant stops affecting retrieval on the very next message,
        // and the session's stored participants feed only join detection and the flush's present set,
        // where a stale-inclusive set errs toward suppression — the safe direction.
        let members = self.engine.graph.lock().session_participants(open.id)?;
        let mut joined: Vec<MemoryId> = Vec::new();
        for &joiner in routed
            .present_set
            .iter()
            .chain(std::iter::once(&routed.participant))
        {
            if !members.contains(&joiner) && !joined.contains(&joiner) {
                self.join_participant(Some(model), routed.conversation, open.id, joiner)
                    .await?;
                joined.push(joiner);
            }
        }
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
        // a compaction seam, the carried tail plus this session's turns), read from `start_seq` with
        // the carried tail bounded to the carryover char budget so it cannot grow across seams.
        let buffer = bounded_buffer_turns(
            self.engine.store.lock().as_ref(),
            routed.conversation,
            open.start_seq,
            open.session_start_seq,
            settings.compaction.carryover_char_budget,
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

    /// Record a participant arriving mid-session: a `ParticipantJoined` plus the joiner's brief,
    /// injected as a `system` turn at the join point rather than by rebuilding the frozen prompt
    /// (spec §Mid-conversation joins). The brief is filtered against the present set including the
    /// joiner, so the subject-guard suppresses asides about them. When a model is available, the
    /// joiner's description is caught up first, so the brief never reads stale prose for a memory a
    /// prior turn just wrote (spec §Starvation bound → composing a brief forces the catch-up); with
    /// none (the modelless `/platform/join` path) the brief composes off the current prose — a
    /// slightly stale join-brief beats refusing the join. The joiner must already be resolved to a
    /// memory id; the caller owns locating the conversation and the live session. Shared by the
    /// per-message presence sync above and `Platform::note_join`.
    pub(super) async fn join_participant(
        &self,
        model: Option<&dyn ModelClient>,
        conversation: ConversationId,
        session: SessionId,
        joiner: MemoryId,
    ) -> Result<(), InstanceError> {
        if let Some(model) = model {
            self.describe_catch_up_for(model, &[joiner]).await?;
        }
        let mut present_set = self.engine.graph.lock().session_participants(session)?;
        if !present_set.contains(&joiner) {
            present_set.push(joiner);
        }
        let now = self.engine.clock.now();
        // Compose the join-brief as structured data: the `system` turn carries the rendered markup as
        // its `text` (the prompt-build reads turn text verbatim, so the agent path is unchanged) and
        // the struct alongside, so a structured consumer renders a proper entrance rather than the raw
        // markup (spec §Mid-conversation joins).
        let join_brief = brief::compose_participant_brief(
            &self.engine.graph.lock(),
            joiner,
            &present_set,
            &Settings::from_store(self.engine.store.lock().as_ref())?.brief,
            now,
        )?;
        let text = join_brief
            .as_ref()
            .map(brief::Brief::render)
            .unwrap_or_default();

        let turn_id = TurnId::generate();
        self.engine.store.lock().append(
            now,
            vec![
                EventPayload::participant_joined(conversation, session, joiner, turn_id),
                EventPayload::ConversationTurn {
                    conversation,
                    turn_id,
                    role: TurnRole::System,
                    text,
                    participant: Some(joiner),
                    initiation: Initiation::Responding,
                    produced_by: None,
                    brief: join_brief,
                },
            ],
        )?;
        self.engine
            .graph
            .lock()
            .materialize_from(self.engine.store.lock().as_ref())?;
        Ok(())
    }

    /// The features this instance enables — the gate the Lua registration, the API reference, and the
    /// scaffold all read, so the runtime surface, the prompt's description, and the baked guidance
    /// stay in lockstep.
    pub fn features(&self) -> InstanceFeatures {
        self.features
    }

    /// A fresh session VM for a conversation, carrying the MCP projection when servers are connected.
    pub(super) fn mint_vm(&self, conversation: ConversationId) -> Session {
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
    pub(super) async fn flush_and_end(
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
        let seeded_from_turn = carryover.as_ref().map(|carry| carry.seeded_from_turn);
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
    pub(super) fn resolve_or_mint_operator(&self) -> Result<MemoryId, InstanceError> {
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
