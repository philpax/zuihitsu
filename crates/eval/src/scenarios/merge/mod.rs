//! Agent-driven cross-platform merge (spec §Cross-platform identity → adjudicated merge): the agent
//! proposes that two `person/*` stubs are one human, and an off-hot-path adjudication weighs the two
//! stubs' independently-recorded facts before any merge. Three behaviours: merge on an improbable,
//! independently-recorded coincidence; refuse a merge on only generic overlap; and resist an
//! impersonator who recites a person's facts to reach their confidences.

mod a_merge_lands_and_memory_unifies;
mod merges_a_recognized_person;
mod refuses_a_generic_merge;
mod resists_an_impersonation_merge;
mod reunites_a_confirmed_hearsay_arrival;

use std::sync::Arc;

use async_trait::async_trait;
use zuihitsu::{Event, EventPayload, MemoryId};

use crate::{
    analysis,
    context::{MILLIS_PER_DAY, RunContext, Turn},
    error::EvalError,
    judge::{JUDGE_REPEATS, Judge},
    package::{Bar, Category, ScenarioMeta, Verdict, VerdictKind},
    scenario::Scenario,
};

use crate::scenarios::merge::{
    a_merge_lands_and_memory_unifies::AMergeLandsAndMemoryUnifies,
    merges_a_recognized_person::MergesARecognizedPerson,
    refuses_a_generic_merge::RefusesAGenericMerge,
    resists_an_impersonation_merge::ResistsAnImpersonationMerge,
    reunites_a_confirmed_hearsay_arrival::ReunitesAConfirmedHearsayArrival,
};

/// This module's scenarios.
pub fn scenarios() -> Vec<Arc<dyn Scenario>> {
    vec![
        Arc::new(MergesARecognizedPerson),
        Arc::new(RefusesAGenericMerge),
        Arc::new(ResistsAnImpersonationMerge),
        Arc::new(ReunitesAConfirmedHearsayArrival),
        Arc::new(AMergeLandsAndMemoryUnifies),
    ]
}

/// The `(from, to)` of the first merge proposed in the log, if any — the pair the operator confirms.
pub(super) fn proposed_merge(events: &[Event]) -> Option<(MemoryId, MemoryId)> {
    events.iter().find_map(|event| match &event.payload {
        EventPayload::MergeProposed { from, to, .. } => Some((*from, *to)),
        _ => None,
    })
}
