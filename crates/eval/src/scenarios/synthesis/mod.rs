//! Synthesis: composing a faithful answer from stored memory — describing a person without leaking
//! a withheld detail (`description`), and arbitrating between contradictory accounts (`arbitration`).

pub(crate) mod arbitration;
pub(crate) mod description;

use std::sync::Arc;

use crate::scenario::Scenario;

/// This category's scenarios, submodule by submodule, in report order.
pub(super) fn scenarios() -> Vec<Arc<dyn Scenario>> {
    [description::scenarios(), arbitration::scenarios()]
        .into_iter()
        .flatten()
        .collect()
}
