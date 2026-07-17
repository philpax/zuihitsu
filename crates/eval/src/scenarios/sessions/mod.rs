//! Sessions: how memory behaves across the seams of a session — compaction and its flush
//! visibility (`compaction`), a cold open resurfacing recent threads (`cold_open`), an idle reopen
//! carrying the prior session's raw-transcript tail (`idle_carryover`), a checkpoint syncing parallel
//! rooms via the timer sweep (`checkpoint`) and via a fresh session opening (`session_open`), lived
//! multi-turn conversations (`conversations`), the join brief handed to a newcomer (`joins`), the
//! initiating speaker's guaranteed brief block (`speaker_brief`), and transcript linking and its
//! audience gate (`transcripts`).

pub(crate) mod checkpoint;
pub(crate) mod cold_open;
pub(crate) mod compaction;
pub(crate) mod conversations;
pub(crate) mod idle_carryover;
pub(crate) mod joins;
pub(crate) mod session_open;
pub(crate) mod speaker_brief;
pub(crate) mod transcripts;

use std::sync::Arc;

use crate::scenario::Scenario;

/// This category's scenarios, submodule by submodule, in report order.
pub(super) fn scenarios() -> Vec<Arc<dyn Scenario>> {
    [
        conversations::scenarios(),
        transcripts::scenarios(),
        joins::scenarios(),
        compaction::scenarios(),
        cold_open::scenarios(),
        idle_carryover::scenarios(),
        checkpoint::scenarios(),
        session_open::scenarios(),
        speaker_brief::scenarios(),
    ]
    .into_iter()
    .flatten()
    .collect()
}
