//! Maintenance-pass scenarios: consolidation, canonical profiles, link cleanup, and attestation
//! privacy and survival.

mod append_dedup;
mod attestation_hidden_confirmation;
mod attestation_survives_retraction;
mod attributed_merge;
mod canonicalize;
mod consolidation;
mod consolidation_privacy;

use std::sync::Arc;

use crate::scenario::Scenario;

/// This module's scenarios.
pub fn scenarios() -> Vec<Arc<dyn Scenario>> {
    let mut scenarios: Vec<Arc<dyn Scenario>> = vec![
        Arc::new(consolidation::ConsolidatesOverlappingEntries),
        Arc::new(consolidation_privacy::ConsolidationPreservesPrivacy),
        Arc::new(canonicalize::NamesPlatformStub),
        Arc::new(canonicalize::AvoidsSpuriousMints),
        Arc::new(canonicalize::SuffixesNameCollision),
        Arc::new(append_dedup::HandlesRepeatedFactGracefully),
    ];
    scenarios.extend(attestation_hidden_confirmation::scenarios());
    scenarios.extend(attestation_survives_retraction::scenarios());
    scenarios.extend(attributed_merge::scenarios());
    scenarios
}
