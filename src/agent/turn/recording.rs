//! The model-interaction recording seam and the shared step loop.

use std::{
    collections::BTreeSet,
    sync::atomic::{AtomicU32, Ordering},
    time::Instant,
};

use sha2::{Digest, Sha256};

use futures_util::StreamExt;
use zuihitsu_core::progress::{ProgressKind, TurnProgress};

use crate::{
    clock::Clock,
    engine::Engine,
    event::{EventPayload, EventSource, ModelPhase, RequestRecord, TurnRole},
    ids::{MemoryId, Seq, TurnId},
    metrics::observe_model_call,
    model::{
        Completion, GenerateDelta, GenerateRequest, GenerateResponse, Message, ModelClient,
        ToolCall, ToolChoice,
    },
    prompt::PromptSectionSpan,
    settings::CaptureLevel,
    store::Store,
};

use super::{
    Steps, TurnError, TurnOutcome,
    record::{TurnRecord, append_turn},
    run::tool_call_id,
    tools::run_tool_call,
};
use crate::ids::ConversationId;

/// The cohesive context every model call needs to write its model-interaction record (spec
/// §Observability): which `conversation` and `turn_id` the call belongs to, and how much to
/// `capture`. Threaded into the step loop and the synthesis pass so each `generate` is recorded
/// uniformly. [`Recording::generate`] is the single chokepoint that times a call and best-effort
/// appends a `ModelCalled`; telemetry never breaks a turn, so an append failure is logged, not
/// propagated.
pub(crate) struct Recording {
    /// The conversation the recorded calls belong to, or `None` for off-conversation background work
    /// (the description catch-up). A `None` recording emits no `ModelCalled` telemetry — there is no
    /// conversation to attribute it to — but the work's own events still carry their `produced_by`.
    pub(crate) conversation: Option<ConversationId>,
    pub(crate) turn_id: TurnId,
    pub(crate) capture: CaptureLevel,
    /// How many model calls this recording has started, counted so a progress frame names which
    /// generation of the turn it belongs to (the console resets its accumulated text per step).
    /// Atomic only for `Sync`'s sake — a recording serves one sequential loop.
    pub(crate) steps_started: AtomicU32,
}

impl Recording {
    /// A fresh recording for one turn (or one background pass), its step counter at zero.
    pub(crate) fn new(
        conversation: Option<ConversationId>,
        turn_id: TurnId,
        capture: CaptureLevel,
    ) -> Recording {
        Recording {
            conversation,
            turn_id,
            capture,
            steps_started: AtomicU32::new(0),
        }
    }

    /// Run one model call, timing it and recording its interaction. The caller passes the
    /// delta-encoded `record` (the request side), since only it owns the per-phase buffer state.
    ///
    /// Every call streams — streaming is the model seam's one transport. Text fragments are
    /// published as ephemeral [`TurnProgress`] frames as they arrive (publishing to no subscriber
    /// is free), and the loop acts only on the stream's terminal response, so everything recorded
    /// below (the timing, the metrics, the `ModelCalled` event) reads a complete single-attempt
    /// response. A `Restarted` marker from the retry wrapper lands durably as a `ModelCallAborted`
    /// carrying the discarded partial, and resets the viewer's accumulation.
    pub(crate) async fn generate(
        &self,
        engine: &Engine,
        model: &dyn ModelClient,
        request: &GenerateRequest,
        phase: ModelPhase,
        record: Option<RequestRecord>,
    ) -> Result<GenerateResponse, crate::model::ModelError> {
        let started = Instant::now();
        let response = self
            .generate_streaming(engine, model, request, phase)
            .await?;
        let duration = started.elapsed();
        // The metrics chokepoint (spec §Observability → metrics): every model call — a turn step, a
        // flush, or a background describe/adjudicate pass — observes its latency and token usage
        // here, so the `/control/metrics` saturation counters are complete. Independent of the
        // `ModelCalled` telemetry event (which is conversation-attributed and capture-gated).
        observe_model_call(duration, &response.usage);
        let duration_ms = duration.as_millis() as u64;
        // Off-conversation background work (`conversation` is `None`) records no interaction event:
        // there is no conversation to file it under, and its product carries its own provenance.
        if self.capture != CaptureLevel::Off
            && let Some(conversation) = self.conversation
        {
            let event = EventPayload::ModelCalled {
                conversation,
                turn_id: self.turn_id,
                phase,
                request_digest: request_digest(request),
                request: record,
                completion: response.completion.clone(),
                reasoning: response.reasoning.clone(),
                finish_reason: response.finish_reason.clone(),
                usage: response.usage,
                duration_ms,
            };
            let now = engine.clock.now();
            if let Err(error) = engine
                .store
                .lock()
                .append(now, EventSource::Agent, vec![event])
            {
                tracing::warn!(%error, "could not record the model-interaction event; the turn continues");
            }
        }
        Ok(response)
    }

    /// Drive the streaming call: publish each text fragment as a progress frame, accumulate the
    /// partials so a discarded attempt can be recorded whole, and return the terminal response. On
    /// a `Restarted` marker the discarded partial lands as a `ModelCallAborted` event (durable
    /// visibility for the retry — off-conversation work skips it, exactly like `ModelCalled`) and
    /// a `restart` progress frame voids the viewer's accumulation. A stream that ends without a
    /// terminal is a client contract violation, surfaced as a non-transient error rather than
    /// silently inventing an empty response. Either failure exit publishes an `abandoned` frame
    /// first: a deferral records no agent `ConversationTurn`, so this marker is a viewer's only
    /// signal to drop the dead generation rather than show it generating forever.
    async fn generate_streaming(
        &self,
        engine: &Engine,
        model: &dyn ModelClient,
        request: &GenerateRequest,
        phase: ModelPhase,
    ) -> Result<GenerateResponse, crate::model::ModelError> {
        let step = self.steps_started.fetch_add(1, Ordering::Relaxed);
        let publish = |kind: ProgressKind, text: String| {
            if let Some(conversation) = self.conversation {
                engine.progress.publish(TurnProgress {
                    conversation,
                    turn_id: self.turn_id,
                    phase,
                    kind,
                    text,
                    step,
                });
            }
        };
        let mut partial_reasoning = String::new();
        let mut partial_reply = String::new();
        let mut stream = model.generate_stream(request).await;
        while let Some(delta) = stream.next().await {
            let delta = match delta {
                Ok(delta) => delta,
                Err(error) => {
                    publish(ProgressKind::Abandoned, error.to_string());
                    return Err(error);
                }
            };
            match delta {
                GenerateDelta::Reasoning(text) => {
                    partial_reasoning.push_str(&text);
                    publish(ProgressKind::Reasoning, text);
                }
                GenerateDelta::Reply(text) => {
                    partial_reply.push_str(&text);
                    publish(ProgressKind::Reply, text);
                }
                GenerateDelta::Restarted { attempt, cause } => {
                    // Gated exactly like `ModelCalled`: at `CaptureLevel::Off` the log records no
                    // deliberation text, discarded or not.
                    if self.capture != CaptureLevel::Off
                        && let Some(conversation) = self.conversation
                    {
                        let aborted = EventPayload::ModelCallAborted {
                            conversation,
                            turn_id: self.turn_id,
                            phase,
                            attempt,
                            cause: cause.clone(),
                            partial_reasoning: std::mem::take(&mut partial_reasoning),
                            partial_reply: std::mem::take(&mut partial_reply),
                        };
                        let now = engine.clock.now();
                        if let Err(error) =
                            engine
                                .store
                                .lock()
                                .append(now, EventSource::Agent, vec![aborted])
                        {
                            tracing::warn!(%error, "could not record the aborted model call; the retry continues");
                        }
                    } else {
                        partial_reasoning.clear();
                        partial_reply.clear();
                    }
                    publish(ProgressKind::Restart, cause);
                }
                GenerateDelta::Finished(response) => return Ok(response),
            }
        }
        let error = crate::model::ModelError::Backend {
            model: model.model_id().to_owned(),
            message: "the stream ended without a terminal response".to_owned(),
            transient: false,
        };
        publish(ProgressKind::Abandoned, error.to_string());
        Err(error)
    }

    /// The delta record for a call: a [`RequestRecord::Base`] for the first call of a phase
    /// (`prev_sent_len` is `None`), otherwise a [`RequestRecord::Continuation`] of the messages
    /// appended since the previous call. `None` unless capturing at [`CaptureLevel::Full`], so the
    /// growing buffer is cloned only when it will be stored.
    pub(crate) fn request_record(
        &self,
        request: &GenerateRequest,
        prev_sent_len: Option<usize>,
        system_sections: &[PromptSectionSpan],
    ) -> Option<RequestRecord> {
        if self.capture != CaptureLevel::Full {
            return None;
        }
        Some(match prev_sent_len {
            None => RequestRecord::Base {
                system: request.system.clone(),
                system_sections: system_sections.to_vec(),
                messages: request.messages.clone(),
                tools: request.tools.clone(),
                tool_choice: request.tool_choice,
                thinking: request.thinking,
            },
            Some(sent) => RequestRecord::Continuation {
                appended_messages: request.messages[sent..].to_vec(),
            },
        })
    }
}

/// A `sha2::Sha256` digest (hex) over the full serialized request, recorded on every `ModelCalled`
/// so a prompt reconstructed from the deltas can be checked against the call actually sent.
fn request_digest(request: &GenerateRequest) -> String {
    let mut hasher = Sha256::new();
    hasher.update(serde_json::to_vec(request).unwrap_or_default());
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// The shared step loop a participant turn and a pre-compaction flush both run: generate, execute
/// `run_lua` blocks, feed their results back, until a terminal or `max_steps`. Records exactly one
/// agent `ConversationTurn` (however it ends) carrying `initiation` and `provenance`, and returns the
/// outcome with the peak prompt-token count observed (the largest the buffer reached mid-loop, which
/// the compaction budget bounds).
pub(crate) async fn run_steps(
    steps: Steps<'_>,
) -> Result<(TurnOutcome, Option<u32>, usize, usize), TurnError> {
    let Steps {
        session,
        model,
        engine,
        system,
        system_sections,
        context,
        mut messages,
        initiation,
        provenance,
        max_steps,
        capture,
    } = steps;
    let conversation = session.conversation();
    let recording = Recording::new(Some(conversation), context.turn_id, capture);
    let tools = vec![super::tools::run_lua_tool()];

    let record_agent_turn =
        |store: &mut dyn Store, clock: &dyn Clock, text: String| -> Result<(), TurnError> {
            append_turn(
                store,
                clock,
                TurnRecord {
                    conversation,
                    turn_id: context.turn_id,
                    role: TurnRole::Agent,
                    text,
                    participant: None,
                    initiation,
                    produced_by: provenance.clone(),
                },
            )
        };

    let mut peak_prompt_tokens: Option<u32> = None;
    let mut steps = 0;
    let mut blocks = 0;
    // The message count sent in the prior step, so each step records only the messages appended
    // since (the buffer is append-only within the loop); `None` until the first call.
    let mut prev_sent_len: Option<usize> = None;
    let outcome = 'cycle: {
        for step_index in 0..max_steps {
            // Nearing-budget nudge: two steps from the bound, tell the model to wrap up — once, not
            // every remaining step. It rides the in-memory step frame as a trailing system message
            // (like the flush instruction), never recorded to the log; it persists into the following
            // step's frame, so it appears exactly once from here on.
            if max_steps >= 2 && step_index == max_steps - 2 {
                messages.push(Message::system(
                    "two steps remain in this turn — finish gathering and answer with what you have.",
                ));
            }
            // On the final step the tools are withdrawn (`ToolChoice::None`) so the model must answer
            // with what it has rather than spend its last step on another tool call. Its text becomes
            // the turn's reply through the normal terminal path; `MaxStepsExceeded` is then only the
            // fallback for a model that still fails to produce text.
            let is_final_step = step_index + 1 == max_steps;
            let request = GenerateRequest {
                system: system.to_owned(),
                messages: messages.clone(),
                tools: tools.clone(),
                tool_choice: if is_final_step {
                    ToolChoice::None
                } else {
                    ToolChoice::Auto
                },
                response_format: None,
                thinking: None,
            };
            let record = recording.request_record(&request, prev_sent_len, system_sections);
            prev_sent_len = Some(messages.len());
            let GenerateResponse {
                completion, usage, ..
            } = recording
                .generate(&engine, model, &request, ModelPhase::Step, record)
                .await?;
            steps += 1;
            peak_prompt_tokens = peak_prompt_tokens.max(usage.prompt_tokens);
            match completion {
                Completion::ToolCalls(calls) => {
                    // Normalize the model's arbitrary call ids to the deterministic scheme the
                    // buffer re-render mints, so the next turn's rebuilt buffer reproduces this
                    // exchange byte for byte — a value-unstable id busts the prefix cache outright
                    // on serving stacks whose chat template tokenizes it.
                    let calls: Vec<ToolCall> = calls
                        .into_iter()
                        .enumerate()
                        .map(|(i, call)| ToolCall {
                            id: tool_call_id(context.turn_id, blocks + i),
                            ..call
                        })
                        .collect();
                    messages.push(Message::assistant_tool_calls(calls.clone()));
                    for call in &calls {
                        let result = run_tool_call(session, &engine, &context, call).await?;
                        blocks += 1;
                        messages.push(Message::tool_result(call.id.clone(), result));
                    }
                }
                Completion::Reply(text) if reply_leaks_special_tokens(&text) => {
                    // The model emitted chat-template special-token markup as reply text (typically
                    // at the forced-answer final step, where `ToolChoice::None` forbids a real tool
                    // call and a weaker model transcribes a pseudo-tool-call instead). Such markup
                    // must never reach a participant, so resample the same request once — a transient
                    // decoding artifact usually differs on resample — and take the retry only if it
                    // comes back a clean reply; anything else falls to the silent terminal.
                    tracing::warn!(
                        malformed = %truncate_for_log(&text),
                        "the model leaked special-token markup in its reply; resampling once"
                    );
                    let retry_record =
                        recording.request_record(&request, prev_sent_len, system_sections);
                    let retry = recording
                        .generate(&engine, model, &request, ModelPhase::Step, retry_record)
                        .await?;
                    steps += 1;
                    peak_prompt_tokens = peak_prompt_tokens.max(retry.usage.prompt_tokens);
                    match retry.completion {
                        Completion::Reply(retry_text)
                            if !reply_leaks_special_tokens(&retry_text) =>
                        {
                            record_agent_turn(
                                engine.store.lock().as_mut(),
                                engine.clock.as_ref(),
                                retry_text.clone(),
                            )?;
                            break 'cycle TurnOutcome::Reply(retry_text);
                        }
                        _ => {
                            tracing::warn!(
                                malformed = %truncate_for_log(&text),
                                "the resampled reply is still malformed or not a plain reply; \
                                 staying silent rather than delivering markup"
                            );
                            record_agent_turn(
                                engine.store.lock().as_mut(),
                                engine.clock.as_ref(),
                                String::new(),
                            )?;
                            break 'cycle TurnOutcome::Silent;
                        }
                    }
                }
                Completion::Reply(text) => {
                    record_agent_turn(
                        engine.store.lock().as_mut(),
                        engine.clock.as_ref(),
                        text.clone(),
                    )?;
                    break 'cycle TurnOutcome::Reply(text);
                }
                Completion::Silent => {
                    record_agent_turn(
                        engine.store.lock().as_mut(),
                        engine.clock.as_ref(),
                        String::new(),
                    )?;
                    break 'cycle TurnOutcome::Silent;
                }
            }
        }
        let surfaced = format!("max steps ({max_steps}) reached without a reply");
        record_agent_turn(
            engine.store.lock().as_mut(),
            engine.clock.as_ref(),
            surfaced,
        )?;
        TurnOutcome::MaxStepsExceeded
    };

    Ok((outcome, peak_prompt_tokens, steps, blocks))
}

/// Whether a reply's text leaks model chat-template special-token markup — the `<|` or `|>`
/// delimiters that wrap a backend's special tokens (`<|tool_call|>`, `<|im_start|>`, and the like).
/// A well-formed reply is plain prose the participant reads; those delimiters only appear when the
/// model has transcribed template scaffolding into its answer, so their presence means the reply is
/// malformed and must not be delivered. The heuristic is deliberately exactly these two two-byte
/// delimiters: it does not parse tool-call shapes, and ordinary code or prose — Lua `..`, `{}`, or a
/// comparison like `a < b || b > c` — never contains `<|` or `|>`, so it does not false-positive.
pub(super) fn reply_leaks_special_tokens(text: &str) -> bool {
    text.contains("<|") || text.contains("|>")
}

/// Clip `text` to a bounded, char-boundary-safe prefix for a log field, so a warn over a malformed
/// reply never dumps the whole (possibly large) completion into the diagnostic stream.
fn truncate_for_log(text: &str) -> String {
    const MAX_CHARS: usize = 200;
    let mut clipped: String = text.chars().take(MAX_CHARS).collect();
    if text.chars().nth(MAX_CHARS).is_some() {
        clipped.push('…');
    }
    clipped
}

/// The distinct memories that gained content (a create or an append) since `cycle_start`, in first-
/// write order. Coalescing here means a memory written several times in the turn regenerates once.
pub(crate) fn collect_written_memories(
    store: &dyn Store,
    cycle_start: Seq,
) -> Result<Vec<MemoryId>, TurnError> {
    let mut seen = BTreeSet::new();
    let mut ordered = Vec::new();
    for event in store.read_from(cycle_start.next())? {
        let id = match event.payload {
            EventPayload::MemoryCreated { id, .. }
            | EventPayload::MemoryContentAppended { id, .. }
            // A rename changes no content, but the description is synthesized under the memory's name,
            // so it must be re-synthesized under the new handle — otherwise it keeps the old name and
            // a renamed person's brief broadcasts a name they no longer go by (spec §Identity →
            // Renaming, deadname-safety).
            | EventPayload::MemoryRenamed { id, .. } => id,
            _ => continue,
        };
        if seen.insert(id) {
            ordered.push(id);
        }
    }
    Ok(ordered)
}
