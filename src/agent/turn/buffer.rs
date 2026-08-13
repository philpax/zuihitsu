//! The live conversational buffer: the turn views the next turn replays as the prompt suffix,
//! and the reads that assemble and bound it (spec §Conversations → the live buffer).

use crate::{agent::turn::*, settings::Settings};

/// The seam note marking a non-empty flush reply as undelivered when it replays into a later turn's
/// buffer. A checkpoint flush is internal bookkeeping — its reply reaches no participant — but a
/// non-empty flush reply is an ordinary agent `ConversationTurn` in the log, so on replay it reads as
/// a sent message. Labelling it in place keeps the recorded content visible while telling the next
/// turn it was never delivered, so the agent does not act on having "said" something no one received.
const UNDELIVERED_FLUSH_NOTE: &str =
    "The agent reply just above was an internal checkpoint note, not delivered to any participant.";

/// One tool-call step within an agent turn: the `run_lua` script the model asked to run and the
/// result it saw back. Reconstructed from `LuaExecuted` events so the next turn's buffer carries the
/// full tool-interaction history — the model sees what it already fetched, searched, or computed
/// and does not re-issue the same call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolStep {
    pub script: String,
    pub result: String,
}

/// One turn replayed into the live buffer — the conversational surface the next turn sees as the
/// prompt suffix. Carries the durable turn text and the `run_lua` steps the agent ran this turn
/// (script + result), so the model re-sees what it already did — what it fetched, searched, or
/// wrote — and does not re-issue it next turn. `seq` and `turn_id` let a compaction mark the
/// carried tail (`seeded_from_turn` and the next buffer's start).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TurnView {
    pub seq: Seq,
    pub turn_id: TurnId,
    pub role: TurnRole,
    pub text: String,
    pub participant: Option<MemoryId>,
    /// When the turn was recorded — the time it is stamped with when replayed (spec §Time → "Now").
    pub recorded_at: Timestamp,
    /// The `run_lua` steps this turn's agent response ran, in order. Empty for participant/system
    /// turns, and for an agent turn that ran no blocks (a direct reply).
    pub steps: Vec<ToolStep>,
    /// The provenance the turn was recorded with — which template drove an agent turn. What lets a
    /// buffer scan recognize a flush turn (its `template_name` is `Flush`) and derive the session's
    /// flush watermark ([`flushed_up_to`]). `None` for participant/system turns and for agent turns
    /// recorded before provenance existed.
    pub produced_by: Option<ProducedBy>,
    /// The prompt size the backend reported at this turn's first model call — what the carryover trim
    /// prices the buffer with ([`carryover_start`]). `None` for participant and system turns (they run
    /// no model call), for an agent turn whose backend reported no usage, and under
    /// a backend that reports no usage.
    pub prompt_tokens: Option<u32>,
    /// The files the turn's message carried, replayed from the payload so a later turn sees the same
    /// attachments the live one did. Empty for every turn that carried none.
    pub attachments: Vec<Attachment>,
}

/// The `conversation`'s `ConversationTurn`s recorded at or after `from_seq`, oldest first — the live
/// buffer the next turn replays as the prompt suffix (spec §Conversations → the live buffer).
/// `from_seq` is the live session's start (so the whole session is read) or a carried tail across a
/// compaction seam (so only the carryover plus the new session's turns are read).
pub fn buffer_turns(
    store: &dyn Store,
    conversation: ConversationId,
    from_seq: Seq,
) -> Result<Vec<TurnView>, StoreError> {
    let mut turns = Vec::new();
    // A turn's `run_lua` blocks commit (and record their `LuaExecuted`) before the agent's reply turn,
    // both stamped with the same `turn_id` — so accumulate each turn's tool-call steps and attach them
    // to that turn's agent `TurnView` when it arrives.
    let mut steps_by_turn: BTreeMap<TurnId, Vec<ToolStep>> = BTreeMap::new();
    // A turn's model calls record before its agent `ConversationTurn`, so the first reported prompt
    // size is in hand by the time the turn arrives. The first call's is the one the carryover trim
    // wants: it measures the prompt as it stood *before* this turn generated anything.
    let mut prompt_tokens_by_turn: BTreeMap<TurnId, u32> = BTreeMap::new();
    for event in store.read_from(from_seq)? {
        match event.payload {
            EventPayload::ModelCalled {
                conversation: turn_conversation,
                turn_id,
                usage,
                ..
            } if turn_conversation == conversation => {
                if let Some(prompt_tokens) = usage.prompt_tokens {
                    prompt_tokens_by_turn
                        .entry(turn_id)
                        .or_insert(prompt_tokens);
                }
            }
            EventPayload::LuaExecuted {
                conversation: turn_conversation,
                turn_id,
                script,
                result,
                terminal_cause,
                ..
            } if turn_conversation == conversation => {
                let result = result.unwrap_or_else(|| {
                    terminal_cause
                        .as_ref()
                        .map(|cause| ToolError::from(cause.clone()).to_string())
                        .unwrap_or_default()
                });
                steps_by_turn
                    .entry(turn_id)
                    .or_default()
                    .push(ToolStep { script, result });
            }
            EventPayload::ConversationTurn {
                conversation: turn_conversation,
                turn_id,
                role,
                text,
                participant,
                produced_by,
                attachments,
                ..
            } if turn_conversation == conversation => {
                let (steps, prompt_tokens) = if role == TurnRole::Agent {
                    (
                        steps_by_turn.remove(&turn_id).unwrap_or_default(),
                        prompt_tokens_by_turn.remove(&turn_id),
                    )
                } else {
                    (Vec::new(), None)
                };
                // A non-empty agent reply produced by the flush path is an internal checkpoint note
                // that reached no participant, yet on replay it is an ordinary agent turn
                // indistinguishable from a sent message. The `Flush` provenance is the tight signal —
                // the same field the flush watermark keys on ([`flushed_up_to`]) — and stricter than
                // `Initiation::Initiated`, which also covers agent-initiated turns that *are*
                // delivered. Mark it undelivered so the next turn does not "remember saying" it; an
                // empty flush reply already replays silently (`buffer_messages` emits no assistant
                // message for empty text) and needs no marker.
                let is_undelivered_flush = role == TurnRole::Agent
                    && !text.is_empty()
                    && produced_by.as_ref().is_some_and(ProducedBy::is_flush);
                turns.push(TurnView {
                    seq: event.seq,
                    turn_id,
                    role,
                    text,
                    participant,
                    recorded_at: event.recorded_at,
                    steps,
                    produced_by,
                    prompt_tokens,
                    attachments,
                });
                // The marker rides right after the flush reply as a system note, in the same style as
                // the supersession seam hint — recorded content stays visible, honestly labelled.
                if is_undelivered_flush {
                    turns.push(TurnView {
                        seq: event.seq,
                        turn_id,
                        role: TurnRole::System,
                        text: UNDELIVERED_FLUSH_NOTE.to_owned(),
                        participant: None,
                        recorded_at: event.recorded_at,
                        steps: Vec::new(),
                        produced_by: None,
                        prompt_tokens: None,
                        attachments: Vec::new(),
                    });
                }
            }
            // The supersession seam hint replays as a system turn at its log position — right after
            // the interrupting participant turn — so the successor is told the earlier message was
            // never answered. Byte-identity reasoning on the payload's doc.
            EventPayload::TurnSuperseded {
                conversation: turn_conversation,
                turn_id,
                text,
            } if turn_conversation == conversation => {
                turns.push(TurnView {
                    seq: event.seq,
                    turn_id,
                    role: TurnRole::System,
                    text,
                    participant: None,
                    recorded_at: event.recorded_at,
                    steps: Vec::new(),
                    produced_by: None,
                    prompt_tokens: None,
                    attachments: Vec::new(),
                });
            }
            // The ambient recall hint replays as a system turn at its log position — the byte-identity
            // reasoning lives on the payload's doc.
            EventPayload::AmbientRecallSurfaced {
                conversation: turn_conversation,
                turn_id,
                text,
                ..
            } if turn_conversation == conversation => {
                turns.push(TurnView {
                    seq: event.seq,
                    turn_id,
                    role: TurnRole::System,
                    text,
                    participant: None,
                    recorded_at: event.recorded_at,
                    steps: Vec::new(),
                    produced_by: None,
                    prompt_tokens: None,
                    attachments: Vec::new(),
                });
            }
            _ => {}
        }
    }
    Ok(turns)
}

/// Read the live buffer ([`buffer_turns`]) and bound its carried tail, so the buffer cannot grow
/// without bound across compaction seams. `session_start_seq` is this session's own `SessionStarted`
/// seq; it splits the read into the carried tail (turns before it, seeded from a prior session across
/// a compaction seam) and this session's own turns (at or after it). The tail is re-trimmed to
/// the carryover budget — the same trim the carryover staging applies ([`carryover_start`]) — so a
/// session seeded from a carryover, and every session after it, sees a tail no larger than the budget
/// rather than every turn accrued since the original carryover point. The session's own turns always
/// ride whole (the compaction budget already bounds them), so the buffer is structurally
/// `≤ token_budget + one session's turns`, regardless of how the budgets are tuned. For a fresh session
/// `start_seq == session_start_seq`, the tail is empty and this is exactly [`buffer_turns`].
pub fn bounded_buffer_turns(
    store: &dyn Store,
    conversation: ConversationId,
    start_seq: Seq,
    session_start_seq: Seq,
    pricing: Pricing,
) -> Result<Vec<TurnView>, StoreError> {
    let mut turns = buffer_turns(store, conversation, start_seq)?;
    // The read is in seq order, so the carried tail is the prefix below this session's own start.
    let split = turns.partition_point(|turn| turn.seq < session_start_seq);
    let keep_from = carryover_start(&turns[..split], pricing);
    turns.drain(..keep_from);
    Ok(turns)
}

/// What the trim needs beyond the buffer: the budget it fills to, and the inlining budget a turn's
/// text attachments were rendered under, since an estimate that ignored them would price a turn
/// carrying a file as though it carried a sentence.
#[derive(Clone, Copy, Debug)]
pub struct Pricing {
    pub carryover_token_budget: i64,
    /// `TurnSettings::max_attachment_text_chars` — the whole message's inlining budget.
    pub attachment_text_chars: usize,
}

impl Pricing {
    /// The pricing an instance's current settings state.
    pub fn of(settings: &Settings) -> Pricing {
        Pricing {
            carryover_token_budget: settings.compaction.carryover_token_budget,
            attachment_text_chars: settings.turn.max_attachment_text_chars.max(0) as usize,
        }
    }
}

/// The estimated price of one turn, for the turns nothing measured.
///
/// Characters over the shared four-per-token rule, counting what the prompt carries rather than what
/// the log records: the turn's text, the tool steps it replays, and what its attachments add. This is
/// the only pricing in the module that guesses, and it exists because a turn that has not been
/// followed by a model call has no reported cost to read.
pub fn estimated_turn_tokens(turn: &TurnView, pricing: Pricing) -> usize {
    let steps: usize = turn
        .steps
        .iter()
        .map(|step| step.script.chars().count() + step.result.chars().count())
        .sum();
    let attachments: usize = turn
        .attachments
        .iter()
        .map(|attachment| match attachment.kind {
            // Inlined up to the message's budget, however long the file itself runs.
            AttachmentKind::Text => {
                (attachment.byte_len as usize).min(pricing.attachment_text_chars)
            }
            AttachmentKind::Image => IMAGE_ESTIMATE_CHARS,
            // Announced by name, type, and size: a line, not a payload.
            AttachmentKind::Opaque => ANNOUNCEMENT_CHARS,
        })
        .sum();
    estimated_tokens_from_chars(turn.text.chars().count() + steps + attachments)
}

/// What one image is guessed at, in the characters this estimate is stated in. A vision backend bills
/// an image at a few hundred to a couple of thousand tokens by its dimensions; this is the middle of
/// that band. It applies only until the next model call reports the real number.
const IMAGE_ESTIMATE_CHARS: usize = 4_000;

/// The announcement line an attachment the model cannot read costs: its name, media type, and size.
const ANNOUNCEMENT_CHARS: usize = 80;

/// The index into `turns` of the oldest turn the carryover can afford, filling backward from the
/// newest — the raw-transcript carryover trim (spec §Compaction → raw-transcript carryover). The
/// newest turn is always kept, even alone over budget. Returns `turns.len()` for an empty slice.
///
/// **The cost is the backend's own count, not a guess.** A turn's first model call records the prompt
/// as it stood before that turn generated, and every prompt is assembled over the same frozen prefix,
/// so the difference between two such counts is exactly what the buffer grew by between them —
/// replayed tool steps, inlined attachments, and image parts included, each charged what the provider
/// charged rather than what a character count imagines. The trim therefore cuts at a turn that made a
/// model call, and asks one question per candidate: does the growth since that turn fit?
///
/// Two things are genuinely unknown and only there does an estimate appear. The newest exchange has no
/// successor to bracket it, so what it costs is not yet reported; and turns before the oldest recorded
/// call were never bracketed at all. A backend that reports no usage leaves the whole buffer unknown
/// — [`estimated_start`] is that case.
pub fn carryover_start(turns: &[TurnView], pricing: Pricing) -> usize {
    let budget = pricing.carryover_token_budget.max(0) as usize;
    let newest = turns.len().saturating_sub(1);
    let measured: Vec<(usize, u32)> = turns
        .iter()
        .enumerate()
        .filter_map(|(idx, turn)| turn.prompt_tokens.map(|tokens| (idx, tokens)))
        .collect();
    let Some(&(last, last_tokens)) = measured.last() else {
        return estimated_start(turns, pricing);
    };

    // Everything from the newest recorded call onward — that turn's own generation and any turn after
    // it — is unreported until the next call, and rides along whatever else is carried.
    let tail: usize = turns[last..]
        .iter()
        .map(|turn| estimated_turn_tokens(turn, pricing))
        .sum();
    // The newest turn always rides, even alone over budget.
    let mut start = newest;
    // Carrying from a recorded call costs the growth since it, exactly: both prompts were measured
    // over the same frozen prefix, so their difference is the buffer content between them.
    for &(idx, tokens) in measured.iter().rev() {
        if last_tokens.saturating_sub(tokens) as usize + tail > budget {
            break;
        }
        start = idx.min(start);
    }
    // Reaching past the oldest recorded call means paying for turns nothing bracketed.
    if let Some(&(oldest, oldest_tokens)) = measured.first()
        && start == oldest
        && oldest > 0
    {
        let unbracketed: usize = turns[..oldest]
            .iter()
            .map(|turn| estimated_turn_tokens(turn, pricing))
            .sum();
        if last_tokens.saturating_sub(oldest_tokens) as usize + tail + unbracketed <= budget {
            start = 0;
        }
    }
    start
}

/// The trim with nothing measured to go on: the newest-first fill over [`estimated_turn_tokens`], for
/// a buffer whose turns recorded no model call at all.
fn estimated_start(turns: &[TurnView], pricing: Pricing) -> usize {
    let budget = pricing.carryover_token_budget.max(0) as usize;
    let mut total = 0usize;
    let mut start = turns.len();
    for (idx, turn) in turns.iter().enumerate().rev() {
        let next = total.saturating_add(estimated_turn_tokens(turn, pricing));
        if start != turns.len() && next > budget {
            break;
        }
        total = next;
        start = idx;
    }
    start
}

/// The distinct memory IDs the `conversation`'s blocks touched (read or wrote) from `from_seq`,
/// unioned across its `LuaExecuted` events in first-touch order — the touch-derived working set
/// carried across a compaction seam (spec §Compaction → working-set carryover). The read half is as
/// valuable as the write half: the agent looked something up because it was relevant.
pub fn session_touched(
    store: &dyn Store,
    conversation: ConversationId,
    from_seq: Seq,
) -> Result<Vec<MemoryId>, StoreError> {
    let mut seen = BTreeSet::new();
    let mut ordered = Vec::new();
    for event in store.read_from(from_seq)? {
        if let EventPayload::LuaExecuted {
            conversation: block_conversation,
            touched,
            ..
        } = event.payload
            && block_conversation == conversation
        {
            for id in touched {
                if seen.insert(id) {
                    ordered.push(id);
                }
            }
        }
    }
    Ok(ordered)
}

/// The distinct memory IDs recently touched across *every* conversation, most-recent-first — the
/// cold-open analogue of the working-set carryover, for a session that opens without one (after an
/// idle gap, or on first contact). It scans the `LuaExecuted` events recorded at or after `since`,
/// unioning their `touched` sets in reverse-chronological first-touch order so the freshest thread
/// ranks first and survives the brief's char budget, and caps the result at `limit`. The read half
/// is as valuable as the write half, exactly as for the carryover: the agent looked something up
/// because it was relevant. Cross-conversation privacy is not the concern here — every candidate is
/// re-filtered through the visibility predicate against the opening session's present set when the
/// brief renders it, so a thread from another room surfaces only what that audience may see. `limit`
/// of `0` yields nothing, disabling the cold-open derivation.
pub fn recent_touched(
    store: &dyn Store,
    since: Timestamp,
    limit: usize,
) -> Result<Vec<MemoryId>, StoreError> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let events = store.read_from(Seq::ZERO)?;
    let mut seen = BTreeSet::new();
    let mut ordered = Vec::new();
    for event in events.into_iter().rev() {
        if event.recorded_at.as_millisecond() < since.as_millisecond() {
            continue;
        }
        if let EventPayload::LuaExecuted { touched, .. } = event.payload {
            for id in touched {
                if seen.insert(id) {
                    ordered.push(id);
                    if ordered.len() == limit {
                        return Ok(ordered);
                    }
                }
            }
        }
    }
    Ok(ordered)
}

/// The session's flush watermark, derived from the log: the seq of the buffer's last flush turn — an
/// agent turn whose `produced_by` carries the `Flush` template, a checkpoint or a prior session's
/// end-flush riding the carried tail — or `session_start` when no flush turn is in view. Everything at
/// or before the watermark has been flushed to memory; the turns past it are the unflushed delta a
/// checkpoint flush scopes itself to (spec §Compaction → checkpoint flush). Derived per read rather
/// than held as mutable session state, so replaying the log reproduces it exactly.
pub fn flushed_up_to(buffer: &[TurnView], session_start: Seq) -> Seq {
    buffer
        .iter()
        .rev()
        .find(|turn| turn.produced_by.as_ref().is_some_and(ProducedBy::is_flush))
        .map(|turn| turn.seq)
        .unwrap_or(session_start)
}
