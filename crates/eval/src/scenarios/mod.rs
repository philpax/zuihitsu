//! The scenario registry. The set grows over time (spec §Validation → the corpus is meant to grow).
//! Each module owns its own scenarios; `all()` is their composition, in report order grouped by the
//! surface each exercises.

mod arbitration;
mod checkpoint;
mod compaction;
mod content_limit;
mod conversations;
mod decay;
mod description;
mod identity;
mod joins;
mod merge;
mod privacy;
mod recall;
mod relations;
mod rename;
mod reuse;
mod scheduling;
mod tagging;
mod temporal;
mod transcripts;
mod write_honesty;

use std::sync::Arc;

use crate::scenario::Scenario;

/// Every scenario the harness knows, in report order.
pub fn all() -> Vec<Arc<dyn Scenario>> {
    [
        recall::scenarios(),
        transcripts::scenarios(),
        reuse::scenarios(),
        tagging::scenarios(),
        relations::scenarios(),
        merge::scenarios(),
        identity::scenarios(),
        rename::scenarios(),
        decay::scenarios(),
        scheduling::scenarios(),
        temporal::scenarios(),
        arbitration::scenarios(),
        privacy::scenarios(),
        joins::scenarios(),
        description::scenarios(),
        compaction::scenarios(),
        checkpoint::scenarios(),
        conversations::scenarios(),
        write_honesty::scenarios(),
        content_limit::scenarios(),
    ]
    .into_iter()
    .flatten()
    .collect()
}
