//! `run_turn` and `run_flush`: the two entry points that assemble the system prompt and drive the
//! step loop.

use std::collections::{BTreeMap, HashSet};

use crate::{
    engine::Engine,
    event::{
        EventPayload, EventSource, Initiation, ProducedBy, PromptTemplateName, Teller, TurnRole,
    },
    ids::{MemoryId, TurnId},
    memory::memory_block::Authority,
    metrics::observe_turn_deferred,
    model::Message,
    time::Timestamp,
};

use super::{
    BlockContext, Flush, Steps, Turn, TurnError, TurnOutcome, TurnReport,
    buffer::TurnView,
    record::{TurnRecord, append_turn},
    recording::run_steps,
    tools::full_api_reference,
};
use crate::agent::{system_prompt, templates};

/// Run one turn: record the inbound participant message, then loop model steps until a terminal.
pub async fn run_turn(turn: Turn<'_>) -> Result<TurnReport, TurnError> {
    let Turn {
        session,
        model,
        engine,
        inbound,
        inbound_participant,
        brief,
        session_started_at,
        buffer,
        template,
        authority,
        present_set,
        brief_memories,
        ambient,
        max_steps,
        block_timeout,
        max_block_attempts,
        max_entry_chars,
        capture,
    } = turn;
    let conversation = session.conversation();
    // Content the agent writes this turn is attributed to the speaker by default (an append opts out
    // with `by_agent` for the agent's own observations — see `mem:append`).
    let teller = Teller::Participant(inbound_participant);
    // The participant's inbound turn id — generated once, recorded on the `ConversationTurn`, and
    // returned in the `TurnReport` so a platform client can map its message id to the turn id.
    let participant_turn_id = TurnId::generate();
    // An inbound participant message is not inference, so it carries no provenance.
    append_turn(
        engine.store.lock().as_mut(),
        engine.clock.as_ref(),
        TurnRecord {
            conversation,
            turn_id: participant_turn_id,
            role: TurnRole::Participant,
            text: inbound.to_owned(),
            participant: Some(inbound_participant),
            initiation: Initiation::Responding,
            produced_by: None,
        },
    )?;

    // Assemble the frozen system prompt once for the cycle: the `template` framing (Scaffold for a
    // participant turn, Imprint for the interview), the agent's identity from `self`, and the time.
    let framing = templates::latest_template(engine.store.lock().as_ref(), template)?;
    let framing_version = framing.as_ref().map(|t| t.version);
    let framing_body = framing.map(|t| t.body).unwrap_or_default();
    let (identity, vocabulary) = {
        let graph = engine.graph.lock();
        let identity = match graph.self_memory()? {
            Some(self_memory) => graph.entries_local(self_memory.id)?,
            None => Vec::new(),
        };
        let vocabulary =
            system_prompt::render_vocabulary(&graph.all_tags()?, &graph.all_relations()?);
        (identity, vocabulary)
    };
    // The API description is build-derived: rendered from the running binary so the prompt and the
    // installed Lua API can't drift (spec §System prompt → API description), plus the connected MCP
    // servers' projected tools (runtime-derived from the session's catalogue). The tag vocabulary is
    // runtime data, read from the graph above and rendered alongside the API description.
    let api_reference = full_api_reference(session);
    // The time is frozen to the session's start, not the live clock: every turn in the session then
    // sends an identical system prefix (current time rides in the per-message stamps below), so the
    // serving layer can reuse its prefix cache across the session rather than re-encoding on each turn.
    let assembled = system_prompt::assemble(
        &framing_body,
        &identity,
        &api_reference,
        &vocabulary,
        brief,
        session_started_at,
    );

    // Provenance for the agent's turn: the chat model and the template it ran against. If the
    // template isn't registered (it always is post-genesis), the attribution is simply absent.
    let agent_provenance = framing_version.map(|version| ProducedBy {
        model_id: model.model_id().into(),
        template_name: template,
        template_version: version,
    });

    // The agent's whole response cycle shares one turn id; its blocks stamp their events with it. The
    // live buffer is replayed as the prompt suffix, then the current inbound message.
    let turn_id = TurnId::generate();
    let names = participant_names(engine.as_ref(), buffer, &[inbound_participant]);
    let mut messages = buffer_messages(buffer, &names);
    messages.push(Message::user(stamp(
        inbound,
        engine.clock.now(),
        names.get(&inbound_participant).map(String::as_str),
    )));

    // Ambient recall: a fast lexical pass over the inbound message surfaces memories the frozen brief
    // did not, injected as a system message so the agent recalls what it would not have thought to
    // search for (spec §Conversations and briefs → ambient recall). It rides after the inbound, so it
    // reads as a note about that message. The `AmbientRecallSurfaced` event is recorded right after the
    // inbound turn and carries the rendered hint verbatim, so the buffer read path replays it in the
    // same position next turn — the prompt stays byte-identical and the serving layer's prefix cache
    // survives. The `now` is captured once and shared by the event and the live message stamp, so the
    // replay (which stamps with the event's recorded time) reproduces this message exactly.
    let hint = {
        let graph = engine.graph.lock();
        let exclude: HashSet<MemoryId> =
            present_set.iter().chain(brief_memories).copied().collect();
        super::ambient::ambient_recall(
            &graph,
            &ambient,
            inbound,
            &exclude,
            session.features().transcripts,
            session.features().browsing,
        )?
    };
    if let Some(hint) = hint {
        let now = engine.clock.now();
        engine.store.lock().append(
            now,
            EventSource::Orchestration,
            vec![EventPayload::ambient_recall_surfaced(
                conversation,
                turn_id,
                hint.message.clone(),
                hint.hits,
            )],
        )?;
        messages.push(Message::system(stamp(&hint.message, now, None)));
    }

    let steps_result = run_steps(Steps {
        session,
        model,
        engine: engine.clone(),
        system: assembled.text(),
        system_sections: assembled.sections(),
        context: BlockContext {
            teller,
            authority,
            turn_id,
            block_timeout,
            max_block_attempts,
            max_entry_chars,
            present_set: present_set.to_vec(),
            dry_run: false,
        },
        messages,
        initiation: Initiation::Responding,
        provenance: agent_provenance,
        max_steps,
        capture,
    })
    .await;
    let (outcome, peak_prompt_tokens, steps, blocks) = match steps_result {
        Ok(resolved) => resolved,
        // The model backend is unreachable (retries, if any, exhausted by the wrapper, or the
        // circuit open): defer the turn instead of erroring it. The inbound participant turn was
        // appended above, before the loop, so nothing durable is lost — and deliberately no agent
        // turn is recorded (the harness's retries are infra-transparent, spec §Event sourcing:
        // they emit nothing to the log). The report's `turn_id` therefore keys no events, and the
        // step/block counts read zero even if the loop ran partial steps before the outage —
        // those blocks' events are in the log under this turn id, but with no agent turn to
        // anchor them the buffer replay carries only the inbound. Lua/store/graph failures keep
        // the error path: `Deferred` is only for model-transport failure.
        Err(TurnError::Model(error)) if error.is_unavailable() => {
            tracing::warn!(%error, "the model backend is unreachable; deferring the turn");
            observe_turn_deferred();
            (TurnOutcome::Deferred, None, 0, 0)
        }
        Err(error) => return Err(error),
    };

    // Description regeneration and temporal extraction for the memories this turn wrote run off the hot
    // path, in the background describer (spec §Write path → regenerate off the hot path, as a
    // catch-up), so the reply is not held waiting on summarization. The entries are committed and
    // readable now; only the synthesized description lags until the next catch-up.

    Ok(TurnReport {
        outcome,
        prompt_tokens: peak_prompt_tokens,
        steps,
        blocks,
        turn_id,
        participant_turn_id,
    })
}

/// Run the budget-gated pre-compaction flush: one agent turn whose job is to write durable working
/// state to memory before the session is cut (spec §Compaction). It reuses the session's scaffold
/// system prompt and appends the `Flush` template's instruction as a trailing system message, so the
/// cached system-plus-buffer prefix is preserved rather than re-encoded. It sees the full session
/// buffer, acts unprompted (`Initiation::Initiated`), and attributes its writes to the agent. An
/// ordinary `ConversationTurn` + `LuaExecuted`, fully logged and replay-trivial. A no-op if no `Flush`
/// template is registered (an agent born before the template shipped).
pub(crate) async fn run_flush(flush: Flush<'_>) -> Result<(), TurnError> {
    let Flush {
        session,
        model,
        engine,
        brief,
        session_started_at,
        buffer,
        present_set,
        max_steps,
        block_timeout,
        max_block_attempts,
        max_entry_chars,
        capture,
    } = flush;
    // The flush's standing instruction comes from the `Flush` template; without it there is nothing to
    // flush. It rides as a trailing message (below), not as the system prompt.
    let Some(flush_instruction) =
        templates::latest_template(engine.store.lock().as_ref(), PromptTemplateName::Flush)?
    else {
        return Ok(());
    };
    // Frame the flush with the SAME scaffold system prompt the session's live turns used, so the
    // identical system-plus-buffer prefix is already in the serving layer's cache. Swapping in a
    // distinct flush system prompt would change token zero and force a full re-encode of the whole
    // buffer at max context — the worst-case latency on the hot path.
    let scaffold =
        templates::latest_template(engine.store.lock().as_ref(), PromptTemplateName::Scaffold)?
            .map(|template| template.body)
            .unwrap_or_default();

    let (identity, vocabulary) = {
        let graph = engine.graph.lock();
        let identity = match graph.self_memory()? {
            Some(self_memory) => graph.entries_local(self_memory.id)?,
            None => Vec::new(),
        };
        let vocabulary =
            system_prompt::render_vocabulary(&graph.all_tags()?, &graph.all_relations()?);
        (identity, vocabulary)
    };
    let api_reference = full_api_reference(session);
    let assembled = system_prompt::assemble(
        &scaffold,
        &identity,
        &api_reference,
        &vocabulary,
        brief,
        session_started_at,
    );
    // The turn is still a flush for provenance — the `Flush` instruction drove it — even though the
    // scaffold now frames the system prompt.
    let provenance = Some(ProducedBy {
        model_id: model.model_id().into(),
        template_name: PromptTemplateName::Flush,
        template_version: flush_instruction.version,
    });

    let turn_id = TurnId::generate();
    // The buffer is the flush's whole context; the flush instruction is appended as a trailing
    // system-role message — a stronger reframing than a user turn, while leaving the cached prefix
    // intact. (If a serving backend rejects a non-leading system message, switch this to
    // `Message::user`.)
    let mut messages = buffer_messages(buffer, &participant_names(engine.as_ref(), buffer, &[]));
    messages.push(Message::system(flush_instruction.body));

    run_steps(Steps {
        session,
        model,
        engine: engine.clone(),
        system: assembled.text(),
        system_sections: assembled.sections(),
        // The flush's writes are the agent's own synthesis, not attributed to any participant. It
        // runs under platform authority — the flush of a platform conversation must not write `self`.
        context: BlockContext {
            teller: Teller::Agent,
            authority: Authority::Platform,
            turn_id,
            block_timeout,
            max_block_attempts,
            max_entry_chars,
            present_set: present_set.to_vec(),
            dry_run: false,
        },
        messages,
        initiation: Initiation::Initiated,
        provenance,
        max_steps,
        capture,
    })
    .await?;

    // As with an ordinary turn, the flush's writes are regenerated off the hot path by the background
    // describer (spec §Write path) — the flush stays cheap, and the post-compaction brief forces the
    // catch-up for the working set before it composes (spec §Starvation bound).
    Ok(())
}

/// Replay the live buffer as chat messages: prior turns mapped to their roles (participant→user,
/// agent→assistant, system→system), skipping empty agent turns (silent terminals). The frozen brief
/// stays in the system prefix only — the buffer never perturbs it (prefix-cache stability). The
/// messages the agent *reads* — participant and system turns — are prefixed with the time they were
/// The deterministic id for a turn's `index`-th tool call, shared by the live step loop (which
/// normalizes the model's arbitrary ids to it) and the buffer re-render (which mints it from the
/// logged steps) — so a re-sent exchange reproduces the live one byte for byte. The index counts
/// tool calls across the whole turn, matching the one-`LuaExecuted`-per-call order the re-render
/// reads back.
pub(crate) fn tool_call_id(turn_id: TurnId, index: usize) -> String {
    format!("call_{}_{}", turn_id.0, index)
}

/// recorded; its own turns are left unstamped so it never learns to emit timestamps (spec §Time).
pub(crate) fn buffer_messages(
    buffer: &[TurnView],
    names: &BTreeMap<MemoryId, String>,
) -> Vec<Message> {
    let mut messages: Vec<Message> = Vec::with_capacity(buffer.len() + 1);
    for buffered in buffer {
        match buffered.role {
            TurnRole::Participant => {
                // Label the turn with who spoke, so a group room is not flattened into an anonymous
                // `user` stream the model cannot attribute (see `participant_names`).
                let speaker = buffered
                    .participant
                    .and_then(|id| names.get(&id))
                    .map(String::as_str);
                messages.push(Message::user(stamp(
                    &buffered.text,
                    buffered.recorded_at,
                    speaker,
                )))
            }
            TurnRole::Agent => {
                // Re-play the turn's tool-call steps so the model re-sees what it already ran —
                // the scripts, the results, the fetched pages and search hits — and does not
                // re-issue them. Each step is an assistant tool-call message followed by its
                // tool-result, matching the within-turn message order the model produced. The ids
                // reproduce what the live turn sent (the step loop normalizes to the same scheme),
                // so the re-rendered exchange is byte-identical and the prefix cache survives on
                // serving stacks whose template tokenizes the id.
                for (i, step) in buffered.steps.iter().enumerate() {
                    let call_id = tool_call_id(buffered.turn_id, i);
                    messages.push(Message::assistant_tool_calls(vec![
                        crate::model::ToolCall {
                            id: call_id.clone(),
                            name: "run_lua".to_owned(),
                            arguments: serde_json::json!({ "script": step.script }).to_string(),
                        },
                    ]));
                    messages.push(Message::tool_result(call_id, step.result.clone()));
                }
                if !buffered.text.is_empty() {
                    messages.push(Message::assistant(buffered.text.clone()));
                }
            }
            TurnRole::System => messages.push(Message::system(stamp(
                &buffered.text,
                buffered.recorded_at,
                None,
            ))),
        }
    }
    messages
}

/// The display name (memory handle, e.g. `person/erin`) of every participant in `buffer` and any
/// `extra` ids, resolved against the graph. Without these, every participant turn renders as an
/// anonymous `user` message, so in a multi-party room the model cannot tell who said what — it reads
/// two speakers as one interlocutor and attributes one's words to the other (the source of the
/// fixture-18 leak). The handle matches `teller_display`, so a brief's "told by person/erin" and a
/// buffer turn's "person/erin:" name the same person.
pub(crate) fn participant_names(
    engine: &Engine,
    buffer: &[TurnView],
    extra: &[MemoryId],
) -> BTreeMap<MemoryId, String> {
    let graph = engine.graph.lock();
    let mut names = BTreeMap::new();
    for id in buffer
        .iter()
        .filter_map(|turn| turn.participant)
        .chain(extra.iter().copied())
    {
        names.entry(id).or_insert_with(|| {
            graph
                .memory_by_id(id)
                .ok()
                .flatten()
                .map(|memory| speaker_display(memory.name.as_str()))
                .unwrap_or_else(|| "someone".to_owned())
        });
    }
    names
}

/// A participant's conversational display name: the [`Namespace::Person`] prefix and any `@platform`
/// stub suffix stripped, so a turn reads `dave:`, not `person/dave@discord:`. The platform suffix is
/// operational noise irrelevant to who is speaking.
pub(super) fn speaker_display(memory_name: &str) -> String {
    let handle = memory_name
        .strip_prefix(crate::ids::Namespace::Person.prefix())
        .unwrap_or(memory_name);
    handle.split('@').next().unwrap_or(handle).to_owned()
}

/// Prefix a message the agent reads with the compact wall-clock time it was recorded (spec §Time →
/// "Now"), and — for a participant turn — who spoke, so the model can attribute statements in a
/// multi-party room.
pub(super) fn stamp(text: &str, at: Timestamp, speaker: Option<&str>) -> String {
    match speaker {
        Some(name) => format!("[{}] {}: {}", crate::time::format_stamp(at), name, text),
        None => format!("[{}] {}", crate::time::format_stamp(at), text),
    }
}
