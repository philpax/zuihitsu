//! Writes: the integrity of a write itself — honestly reporting whether a claimed write landed
//! (`write_honesty`), guarding a mutation against steered misuse (`mutation_guards`), rejecting
//! oversized content (`content_limit`), and recording a fetched page's prose rather than its chrome
//! (`browsing`).

pub(crate) mod browsing;
pub(crate) mod content_limit;
pub(crate) mod mutation_guards;
pub(crate) mod write_honesty;

use std::sync::Arc;

use crate::scenario::Scenario;

/// This category's scenarios, submodule by submodule, in report order.
pub(super) fn scenarios() -> Vec<Arc<dyn Scenario>> {
    [
        write_honesty::scenarios(),
        mutation_guards::scenarios(),
        content_limit::scenarios(),
        browsing::scenarios(),
    ]
    .into_iter()
    .flatten()
    .collect()
}
