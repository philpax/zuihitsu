//! Time: scheduled and recurring wake-ups (`scheduling`), honest anchoring of relative plans and
//! authored dates (`temporal`), and volatile facts going stale (`decay`).

pub(crate) mod decay;
pub(crate) mod scheduling;
pub(crate) mod temporal;

use std::sync::Arc;

use crate::scenario::Scenario;

/// This category's scenarios, submodule by submodule, in report order.
pub(super) fn scenarios() -> Vec<Arc<dyn Scenario>> {
    [
        scheduling::scenarios(),
        temporal::scenarios(),
        decay::scenarios(),
    ]
    .into_iter()
    .flatten()
    .collect()
}
