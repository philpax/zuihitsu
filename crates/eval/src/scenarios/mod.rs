//! The scenario registry. The set grows over time (spec §Validation → the corpus is meant to grow).
//! Each top-level module is a `Category` (`crates/eval/src/package.rs`) and owns the composition of
//! its own submodules; `all()` sorts the composed list by category, so execution and report order
//! both fill the console's category groups contiguously.

mod identity;
mod maintenance;
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

/// Every scenario the harness knows: the modules' own lists, concatenated and then stable-sorted by
/// [`Category`] (`crates/eval/src/package.rs`) order. The sort is load-bearing: a module may
/// contribute scenarios under another module's category (the maintenance module spans several), so
/// composition order alone cannot keep each console category group contiguous — the stable sort
/// does, while preserving each module's internal order within a category.
pub fn all() -> Vec<Arc<dyn Scenario>> {
    let mut scenarios: Vec<Arc<dyn Scenario>> = [
        recall::scenarios(),
        identity::scenarios(),
        relations::scenarios(),
        tagging::scenarios(),
        time::scenarios(),
        privacy::scenarios(),
        sessions::scenarios(),
        writes::scenarios(),
        synthesis::scenarios(),
        maintenance::scenarios(),
    ]
    .into_iter()
    .flatten()
    .collect();
    scenarios.sort_by_key(|scenario| scenario.meta().category);
    scenarios
}
