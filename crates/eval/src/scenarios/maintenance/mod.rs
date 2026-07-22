//! Maintenance-pass scenarios: consolidation, canonical profiles, and link cleanup.

mod consolidation;
mod dedup_at_append;

use std::sync::Arc;

use crate::scenario::Scenario;

/// This module's scenarios.
pub fn scenarios() -> Vec<Arc<dyn Scenario>> {
    vec![
        Arc::new(consolidation::ConsolidatesOverlappingEntries),
        Arc::new(dedup_at_append::RejectsSemanticDuplicate),
    ]
}
