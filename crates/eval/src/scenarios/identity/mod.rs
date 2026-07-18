//! Identity: keeping one person one record across the ways their identity is asserted — an
//! operator-confirmed cross-platform merge (`merge`), a second name landing on the existing operator
//! profile (`operator`), and a rename that must hold on later recall (`rename`) — and keeping the agent's
//! own charter one record across the imprint, not copied back onto `self` (`charter`).

pub(crate) mod charter;
pub(crate) mod merge;
pub(crate) mod operator;
pub(crate) mod rename;

use std::sync::Arc;

use crate::scenario::Scenario;

/// This category's scenarios, submodule by submodule, in report order.
pub(super) fn scenarios() -> Vec<Arc<dyn Scenario>> {
    [
        charter::scenarios(),
        merge::scenarios(),
        operator::scenarios(),
        rename::scenarios(),
    ]
    .into_iter()
    .flatten()
    .collect()
}
