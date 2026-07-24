//! Agent-driven cross-platform merge (spec §Cross-platform identity): the agent proposes that two
//! `person/*` stubs are one human, and the proposal pends for the operator to confirm before any merge
//! lands. Four behaviors: propose a merge on an improbable, independently-recorded coincidence (leaving
//! it for the operator); carry a confirmed merge through so recall unifies; refuse a merge on only
//! generic overlap; and resist an impersonator who recites a person's facts to reach their confidences.

mod a_merge_lands_and_memory_unifies;
mod proposes_a_recognized_merge;
mod records_a_class_fact_on_the_designated_primary;
mod refuses_a_generic_merge;
mod resists_an_impersonation_merge;

use std::{collections::BTreeSet, sync::Arc};

use async_trait::async_trait;
use zuihitsu::{
    Event, EventPayload, LinkPosture, LinkSource, MemoryId, MemoryName, RelationName,
    TEST_PLATFORM, TEST_PLATFORM_ALT, Visibility,
};

use crate::{
    analysis,
    context::MILLIS_PER_DAY,
    judge::{JUDGE_REPEATS, Judge},
    package::{Bar, Category, ScenarioMeta, Verdict, VerdictKind, verdict_from_judge_outcome},
    scenario::Scenario,
    scenarios::identity::merge::{
        a_merge_lands_and_memory_unifies::AMergeLandsAndMemoryUnifies,
        proposes_a_recognized_merge::ProposesARecognizedMerge,
        records_a_class_fact_on_the_designated_primary::RecordsAClassFactOnTheDesignatedPrimary,
        refuses_a_generic_merge::RefusesAGenericMerge,
        resists_an_impersonation_merge::ResistsAnImpersonationMerge,
    },
    step::{EvalStep, OnMissing, Turn},
};

/// This module's scenarios.
pub fn scenarios() -> Vec<Arc<dyn Scenario>> {
    vec![
        Arc::new(ProposesARecognizedMerge),
        Arc::new(RefusesAGenericMerge),
        Arc::new(ResistsAnImpersonationMerge),
        Arc::new(AMergeLandsAndMemoryUnifies),
        Arc::new(RecordsAClassFactOnTheDesignatedPrimary),
    ]
}
