//! Sessions: how memory behaves across the seams of a session — compaction and its flush
//! visibility (`compaction`), a cold open resurfacing recent threads (`cold_open`), a checkpoint
//! syncing parallel rooms (`checkpoint`), lived multi-turn conversations (`conversations`), the
//! join brief handed to a newcomer (`joins`), and transcript linking and its audience gate
//! (`transcripts`).

pub(crate) mod checkpoint;
pub(crate) mod cold_open;
pub(crate) mod compaction;
pub(crate) mod conversations;
pub(crate) mod joins;
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
        checkpoint::scenarios(),
    ]
    .into_iter()
    .flatten()
    .collect()
}
