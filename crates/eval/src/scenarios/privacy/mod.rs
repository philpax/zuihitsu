//! Privacy: keeping a confidence to its audience (`reply_lane`) and withholding a fact from a named
//! party who must not learn it (`exclude`).

pub(crate) mod exclude;
pub(crate) mod reply_lane;

use std::sync::Arc;

use crate::scenario::Scenario;

/// This category's scenarios, submodule by submodule, in report order.
pub(super) fn scenarios() -> Vec<Arc<dyn Scenario>> {
    [reply_lane::scenarios(), exclude::scenarios()]
        .into_iter()
        .flatten()
        .collect()
}
