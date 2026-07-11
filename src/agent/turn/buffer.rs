//! The live conversational buffer: the turn views the next turn replays as the prompt suffix,
//! and the reads that assemble and bound it (spec §Conversations → the live buffer).

use super::*;

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
    for event in store.read_from(from_seq)? {
        match event.payload {
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
                ..
            } if turn_conversation == conversation => {
                let steps = if role == TurnRole::Agent {
                    steps_by_turn.remove(&turn_id).unwrap_or_default()
                } else {
                    Vec::new()
                };
                turns.push(TurnView {
                    seq: event.seq,
                    turn_id,
                    role,
                    text,
                    participant,
                    recorded_at: event.recorded_at,
                    steps,
                    produced_by,
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
/// `char_budget` — the same newest-first fill the carryover staging uses ([`carryover_start`]) — so a
/// session seeded from a carryover, and every session after it, sees a tail no larger than the budget
/// rather than every turn accrued since the original carryover point. The session's own turns always
/// ride whole (the token-budget compaction already bounds them), so the buffer is structurally
/// `≤ char_budget + one session's turns`, regardless of how the budgets are tuned. For a fresh session
/// `start_seq == session_start_seq`, the tail is empty and this is exactly [`buffer_turns`].
pub fn bounded_buffer_turns(
    store: &dyn Store,
    conversation: ConversationId,
    start_seq: Seq,
    session_start_seq: Seq,
    char_budget: i64,
) -> Result<Vec<TurnView>, StoreError> {
    let mut turns = buffer_turns(store, conversation, start_seq)?;
    // The read is in seq order, so the carried tail is the prefix below this session's own start.
    let split = turns.partition_point(|turn| turn.seq < session_start_seq);
    let keep_from = carryover_start(&turns[..split], char_budget);
    turns.drain(..keep_from);
    Ok(turns)
}

/// The index into `turns` of the oldest turn that fits `char_budget`, filling backward from the newest
/// — the raw-transcript carryover trim rule (spec §Compaction → raw-transcript carryover). The newest
/// turn is always kept (even if it alone exceeds the budget), then older turns while their running
/// character total fits. Returns `turns.len()` for an empty slice (an empty tail keeps nothing).
/// Shared by the read-time tail bound ([`bounded_buffer_turns`]) and the carryover staging, so both
/// trim by the same rule.
pub fn carryover_start(turns: &[TurnView], char_budget: i64) -> usize {
    let char_budget = char_budget.max(0) as usize;
    let mut total = 0usize;
    let mut start = turns.len();
    for (idx, turn) in turns.iter().enumerate().rev() {
        let next = total.saturating_add(turn.text.chars().count());
        if start != turns.len() && next > char_budget {
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
        if event.recorded_at.as_millis() < since.as_millis() {
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
        .find(|turn| {
            turn.produced_by
                .as_ref()
                .is_some_and(|produced| produced.template_name == PromptTemplateName::Flush)
        })
        .map(|turn| turn.seq)
        .unwrap_or(session_start)
}
