//! `TurnRecord` and `append_turn`: recording a conversation turn to the event log.

use crate::{
    clock::Clock,
    event::{EventPayload, EventSource, Initiation, ProducedBy, TurnRole},
    ids::{ConversationId, MemoryId, TurnId},
    store::Store,
};

use super::TurnError;

/// One `ConversationTurn` to record: the inbound participant message, the agent's response, or a
/// system message. Holds just the turn's fields; the seams it is written through — the store it is
/// appended to and the clock that stamps it — are passed to [`append_turn`] alongside it.
pub(super) struct TurnRecord {
    pub(super) conversation: ConversationId,
    pub(super) turn_id: TurnId,
    pub(super) role: TurnRole,
    pub(super) text: String,
    /// The speaker of an inbound message; `None` for the agent's own and system turns.
    pub(super) participant: Option<MemoryId>,
    /// Whether the turn responds to a message or is the agent acting unprompted (the pre-compaction
    /// flush is `Initiated`; ordinary participant and agent turns are `Responding`).
    pub(super) initiation: Initiation,
    pub(super) produced_by: Option<ProducedBy>,
}

pub(super) fn append_turn(
    store: &mut dyn Store,
    clock: &dyn Clock,
    record: TurnRecord,
) -> Result<(), TurnError> {
    store.append(
        clock.now(),
        EventSource::Agent,
        vec![EventPayload::ConversationTurn {
            conversation: record.conversation,
            turn_id: record.turn_id,
            role: record.role,
            text: record.text,
            participant: record.participant,
            initiation: record.initiation,
            produced_by: record.produced_by,
            // Only a mid-session join carries a structured brief; the turns this records — inbound,
            // agent reply, flush — do not.
            brief: None,
        }],
    )?;
    Ok(())
}
