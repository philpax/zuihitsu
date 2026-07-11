//! Identity: keeping one person one record across the ways their identity is asserted — an
//! adjudicated cross-platform merge (`merge`), a second name landing on the existing operator
//! profile (`operator`), and a rename that must hold on later recall (`rename`).

pub(crate) mod merge;
pub(crate) mod operator;
pub(crate) mod rename;

use std::sync::Arc;

use crate::scenario::Scenario;

/// This category's scenarios, submodule by submodule, in report order.
pub(super) fn scenarios() -> Vec<Arc<dyn Scenario>> {
    [
        merge::scenarios(),
        operator::scenarios(),
        rename::scenarios(),
    ]
    .into_iter()
    .flatten()
    .collect()
}
