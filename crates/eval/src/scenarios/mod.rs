//! The scenario registry. The set grows over time (spec §Validation → the corpus is meant to grow).
//! Each module owns its own scenarios; `all()` is their composition, in report order grouped by the
//! surface each exercises.

mod arbitration;
mod compaction;
mod description;
mod privacy;
mod recall;
mod relations;
mod scheduling;
mod tagging;

use std::sync::Arc;

use crate::scenario::Scenario;

/// Every scenario the harness knows, in report order.
pub fn all() -> Vec<Arc<dyn Scenario>> {
    [
        recall::scenarios(),
        tagging::scenarios(),
        relations::scenarios(),
        scheduling::scenarios(),
        arbitration::scenarios(),
        privacy::scenarios(),
        description::scenarios(),
        compaction::scenarios(),
    ]
    .into_iter()
    .flatten()
    .collect()
}
