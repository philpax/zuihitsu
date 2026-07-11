//! Recall: writing a fact and reading it back. Cross-room recall by meaning (`across_rooms`), reuse
//! of an existing handle rather than a duplicate (`reuse`), and holding two same-named people apart
//! on read (`name_conflict`).

pub(crate) mod across_rooms;
pub(crate) mod name_conflict;
pub(crate) mod reuse;

use std::sync::Arc;

use crate::scenario::Scenario;

/// This category's scenarios, submodule by submodule, in report order.
pub(super) fn scenarios() -> Vec<Arc<dyn Scenario>> {
    [
        across_rooms::scenarios(),
        reuse::scenarios(),
        name_conflict::scenarios(),
    ]
    .into_iter()
    .flatten()
    .collect()
}
