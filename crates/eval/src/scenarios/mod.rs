//! The scenario registry. The set grows over time (spec §Validation → the corpus is meant to grow).

mod arbitration;
mod compaction;
mod description;
mod privacy;
mod recall;
mod relations;
mod scheduling;
mod tagging;

use std::sync::Arc;

use crate::scenario::Scenario;

/// Every scenario the harness knows, in report order — grouped by the surface each exercises.
pub fn all() -> Vec<Arc<dyn Scenario>> {
    vec![
        Arc::new(recall::Recall),
        Arc::new(tagging::Confidential),
        Arc::new(relations::Knows),
        Arc::new(scheduling::RecurringReminder),
        Arc::new(scheduling::RecurringEmission),
        Arc::new(arbitration::Contradiction),
        Arc::new(privacy::ThirdPartyResidual),
        Arc::new(privacy::FreshSensitiveAside),
        Arc::new(privacy::SensitiveNonPerson),
        Arc::new(description::DescriptionLeak),
        Arc::new(compaction::FlushVisibility),
        Arc::new(compaction::WorkingState),
    ]
}
