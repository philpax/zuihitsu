//! Maintenance-pass scenarios: consolidation, canonical profiles, and link cleanup.

mod canonicalize;
mod consolidation;
mod consolidation_privacy;

use std::sync::Arc;

use crate::scenario::Scenario;

/// This module's scenarios.
pub fn scenarios() -> Vec<Arc<dyn Scenario>> {
    vec![
        Arc::new(consolidation::ConsolidatesOverlappingEntries),
        Arc::new(consolidation_privacy::ConsolidationPreservesPrivacy),
        Arc::new(canonicalize::NamesPlatformStub),
        Arc::new(canonicalize::AvoidsSpuriousMints),
        Arc::new(canonicalize::SuffixesNameCollision),
    ]
}
