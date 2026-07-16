//! Running one session turn: the observability span and the inner turn execution.

use std::time::Duration;

use tracing::Instrument;

use crate::{
    agent::{
        Turn, TurnError, TurnOutcome, TurnRecord, TurnReport, TurnView, append_turn,
        bounded_buffer_turns, run_turn,
    },
    event::{Initiation, PromptTemplateName, TurnRole},
    ids::MemoryId,
    metrics::{observe_turn, observe_turn_error},
    model::ModelClient,
    settings::Settings,
};

use crate::instance::{Instance, InstanceError, session::RoutedTurn};

impl Instance {
    /// Open or continue the session for `conversation`, then run one turn of `inbound` from
    /// `participant` under `template`/`authority`, returning its report and the live buffer it saw
    /// (the buffer the caller's compaction trigger measures). The shared core behind
    /// `Platform::route_message` and `Control::imprint`.
    pub(crate) async fn run_session_turn(
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
            .chain(routed.inbound.iter().map(|m| &m.participant))
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
        let max_entry_chars = settings.memory.max_entry_chars.max(1) as usize;
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
        // Record each inbound participant turn after `ensure_session` opens the session (so the
        // session's `start_seq` precedes them) but after `bounded_buffer_turns` builds the buffer
        // (so the buffer excludes the current turn — it is the current turn, not a prior one). The
        // flush substance gate reads the buffer inside `ensure_session`, before this point, so it
        // sees prior turns but not the current one — matching the old behaviour where `run_turn`
        // recorded the turn after the buffer was built. An inbound participant message is not
        // inference, so it carries no provenance.
        for (msg, &turn_id) in routed.inbound.iter().zip(routed.participant_turn_ids) {
            append_turn(
                self.engine.store.lock().as_mut(),
                self.engine.clock.as_ref(),
                TurnRecord {
                    conversation: routed.conversation,
                    turn_id,
                    role: TurnRole::Participant,
                    text: msg.text.clone(),
                    participant: Some(msg.participant),
                    initiation: Initiation::Responding,
                    produced_by: None,
                },
            )?;
        }
        let report = run_turn(Turn {
            session: &open.vm,
            model,
            engine: self.engine.clone(),
            inbound: routed.inbound,
            participant_turn_ids: routed.participant_turn_ids,
            brief: &open.brief,
            session_started_at: open.started_at,
            buffer: &buffer,
            template,
            authority: routed.authority,
            present_set: routed.present_set,
            brief_memories: &open.brief_memories,
            ambient: settings.ambient.clone(),
            max_steps,
            block_timeout,
            max_block_attempts,
            max_entry_chars,
            capture,
        })
        .await
        .map_err(|error| InstanceError::Turn {
            conversation: Some(routed.conversation),
            error,
        })?;
        Ok((report, buffer))
    }
}
