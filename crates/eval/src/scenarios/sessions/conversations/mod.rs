//! Lived, multi-concern conversations — closer to how the agent is actually used than the focused
//! single-capability fixtures. Each is a multi-turn arc across several rooms and participants that
//! intermingles concerns (relations, scheduling, cross-room recall, privacy, arbitration), and asserts
//! across all of them: the structural outcomes deterministically from the event log, the one judgment
//! that rests on a specific reply through the judge. They categorize as `Sessions` — the whole
//! multi-turn stack across a session's rooms is what they exercise at once.

mod a_reminder_comes_due;
mod a_week_with_the_team;
mod applies_a_remembered_preference;
mod attributed_conflicting_accounts;
mod conflicting_accounts;
mod getting_to_know_someone;
mod shifting_plans;

use std::sync::Arc;

use async_trait::async_trait;
use zuihitsu::{
    EntryId, Event, EventPayload, MemoryId, MemoryName, Namespace, Teller, Timestamp, Visibility,
};

use crate::{
    analysis,
    context::{MILLIS_PER_DAY, RUN_START_MS},
    judge::{JUDGE_REPEATS, Judge},
    package::{Bar, Category, ScenarioMeta, Verdict, VerdictKind},
    scenario::Scenario,
    step::{EvalStep, Turn},
};

use crate::scenarios::sessions::conversations::{
    a_reminder_comes_due::AReminderComesDue, a_week_with_the_team::AWeekWithTheTeam,
    applies_a_remembered_preference::AppliesARememberedPreference,
    attributed_conflicting_accounts::AttributedConflictingAccounts,
    conflicting_accounts::ConflictingAccounts, getting_to_know_someone::GettingToKnowSomeone,
    shifting_plans::ShiftingPlans,
};

/// This module's scenarios.
pub fn scenarios() -> Vec<Arc<dyn Scenario>> {
    vec![
        Arc::new(AWeekWithTheTeam),
        Arc::new(ShiftingPlans),
        Arc::new(AppliesARememberedPreference),
        Arc::new(AReminderComesDue),
        Arc::new(GettingToKnowSomeone),
        Arc::new(ConflictingAccounts),
        Arc::new(AttributedConflictingAccounts),
    ]
}

/// Five days in milliseconds — enough to cross a "this Friday" deadline from the run's Monday anchor.
pub(super) const FIVE_DAYS_MS: i64 = 5 * MILLIS_PER_DAY;
