//! Writes: the integrity of a write itself — honestly reporting whether a claimed write landed
//! (`write_honesty`), guarding a mutation against steered misuse (`mutation_guards`), retracting a
//! mis-filed fact rather than superseding it in place (`retraction`), rejecting oversized content
//! (`content_limit`), recording a fetched page's prose rather than its chrome (`browsing`), and
//! drawing on a file a participant shared rather than on what its sender said about it
//! (`attachments`).

pub(crate) mod attachments;
pub(crate) mod browsing;
pub(crate) mod content_limit;
pub(crate) mod mutation_guards;
pub(crate) mod retraction;
pub(crate) mod write_honesty;

use std::sync::Arc;

use crate::scenario::Scenario;

/// This category's scenarios, submodule by submodule, in report order.
pub(super) fn scenarios() -> Vec<Arc<dyn Scenario>> {
    [
        write_honesty::scenarios(),
        mutation_guards::scenarios(),
        retraction::scenarios(),
        content_limit::scenarios(),
        browsing::scenarios(),
        attachments::scenarios(),
    ]
    .into_iter()
    .flatten()
    .collect()
}
