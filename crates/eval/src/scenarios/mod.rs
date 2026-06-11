//! The scenario registry. The set grows over time (spec §Validation → the corpus is meant to grow).

mod recall;
mod relations;
mod scheduling;
mod tagging;

use std::sync::Arc;

use crate::scenario::Scenario;

/// Every scenario the harness knows, in report order.
pub fn all() -> Vec<Arc<dyn Scenario>> {
    vec![
        Arc::new(recall::Recall),
        Arc::new(tagging::Confidential),
        Arc::new(relations::Knows),
        Arc::new(scheduling::RecurringReminder),
    ]
}
