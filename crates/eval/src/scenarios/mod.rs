//! The scenario registry. The set grows over time (spec §Validation → the corpus is meant to grow).
//! Each top-level module is a `Category` (`crates/eval/src/package.rs`) and owns the composition of
//! its own submodules; `all()` composes the categories in the enum's order, so execution and report
//! order both fill the console's category groups contiguously.

mod identity;
mod privacy;
mod recall;
mod relations;
mod sessions;
mod synthesis;
mod tagging;
mod time;
mod writes;

use std::sync::Arc;

use crate::scenario::Scenario;

/// Every scenario the harness knows: the categories' own lists, concatenated in [`Category`]
/// (`crates/eval/src/package.rs`) order.
pub fn all() -> Vec<Arc<dyn Scenario>> {
    [
        recall::scenarios(),
        identity::scenarios(),
        relations::scenarios(),
        tagging::scenarios(),
        time::scenarios(),
        privacy::scenarios(),
        sessions::scenarios(),
        writes::scenarios(),
        synthesis::scenarios(),
    ]
    .into_iter()
    .flatten()
    .collect()
}
