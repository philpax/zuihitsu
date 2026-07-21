//! Cold-open active threads (spec §Contextual briefs → active threads). A session that opens *cold* —
//! after an idle gap, with no compaction carryover — still re-surfaces the threads a warm continuation
//! would: the memories recent sessions touched are derived into the new brief's `# Active threads`
//! section, each re-filtered through the visibility predicate against the new present set. Without this
//! the section is blank the moment a session opens without a carryover, and the richest re-entry
//! context vanishes exactly when the agent has an empty buffer to work from.
//!
//! The arc: a first session works a concrete infrastructure thread the agent records; the room then
//! goes idle past the gap, so the next message opens a fresh session with no carryover; the re-entry
//! asks vaguely where the thread landed. The structural metric checks the cold session opened with a
//! non-empty active-threads set (the mechanism fired); the judged metric checks the re-entry reply
//! actually carried the resurfaced thread back.

use std::sync::Arc;

use async_trait::async_trait;
use zuihitsu::{Event, TEST_PLATFORM};

use crate::{
    analysis,
    judge::Judge,
    package::{Bar, Category, ScenarioMeta, Verdict, VerdictKind, verdict_from_judge_outcome},
    scenario::Scenario,
    step::{EvalStep, Turn},
};

/// This module's scenarios.
pub fn scenarios() -> Vec<Arc<dyn Scenario>> {
    vec![Arc::new(ColdOpenResurfacesRecentThreads)]
}

/// A concrete decision the first session puts on the record — the thread the cold open should carry
/// back. Probed by meaning, not phrasing.
const DECISION: &str =
    "the checkout-service rewrite moves off the legacy billing module onto the new payments API";

/// A team room worked across an idle seam. In the first session sam locks a concrete decision about
/// the checkout-service rewrite, which the agent records. The room then falls quiet past the idle gap,
/// so the next message opens a *fresh* session — no compaction carryover, an empty buffer. sam returns
/// and asks, vaguely, where the work landed. A warm continuation would have the thread in front of it;
/// a cold open used to open blank. With cold-open active threads, the recently touched memory is
/// derived back into the brief, so the agent re-enters with the thread rather than from nothing.
pub struct ColdOpenResurfacesRecentThreads;

#[async_trait]
impl Scenario for ColdOpenResurfacesRecentThreads {
    fn meta(&self) -> ScenarioMeta {
        ScenarioMeta {
            name: "cold_open_resurfaces_recent_threads".to_owned(),
            category: Category::Sessions,
            description: "A room works a concrete thread, then goes idle past the gap so the next \
                          message opens a fresh session with no carryover and an empty buffer. The \
                          cold open should re-surface the recently worked thread in its brief's \
                          active-threads section, so the vague re-entry recovers where the work \
                          landed. A tracked quality rate, not a safety gate."
                .to_owned(),
            bar: Bar::Metric { threshold: 0.6 },
        }
    }

    fn steps(&self) -> Vec<EvalStep> {
        vec![
            // A first session: sam works a concrete infrastructure decision the agent should record.
            Turn::new(
                TEST_PLATFORM,
                "platform-team",
                "sam",
                "Morning — before standup I want to lock the plan for the checkout-service rewrite. \
                 The core decision: we're moving it off the legacy billing module and onto the new \
                 payments API.",
            )
            .with_present(&["sam"])
            .into(),
            Turn::new(
                TEST_PLATFORM,
                "platform-team",
                "sam",
                "Two constraints worth keeping: the cutover has to land before the November freeze, \
                 and we hold the old endpoints alive for a two-week grace window so mobile can \
                 migrate across.",
            )
            .with_present(&["sam"])
            .into(),
            Turn::new(
                TEST_PLATFORM,
                "platform-team",
                "sam",
                "That's the shape of it — just wanted it on the record. I'll send ticket links \
                 later.",
            )
            .with_present(&["sam"])
            .into(),
            // Settle the description and index catch-up so the state is durable before the seam.
            EvalStep::Settle,
            // The room goes quiet past the idle gap: the next message opens a fresh session, with no
            // compaction carryover — the cold open the active-threads derivation is for.
            EvalStep::AdvancePastIdleGap,
            // sam returns and asks vaguely where the thread landed. With an empty buffer, the re-entry
            // leans on what the cold-open brief re-surfaced.
            Turn::new(
                TEST_PLATFORM,
                "platform-team",
                "sam",
                "Back — remind me where we landed on the checkout-service work?",
            )
            .with_present(&["sam"])
            .into(),
        ]
    }

    async fn assess(&self, events: &[Event], judge: &Judge) -> Vec<Verdict> {
        // The mechanism, structurally: the last session opened is the cold one (session 1 opened on the
        // first turn, the cold session on the return past the idle gap). It should have derived a
        // non-empty active-threads set and rendered the section — the direct test of the fix, model-free.
        let briefs = analysis::session_briefs(events);
        let cold = briefs.last();
        let resurfaced = cold.is_some_and(|(brief, working_set)| {
            !working_set.is_empty() && brief.contains("# Active threads")
        });

        // The behaviour, by meaning: the vague re-entry recovered the recorded decision rather than
        // asking sam to restate it — the benefit the resurfaced thread buys on an empty buffer.
        let reply = analysis::last_agent_reply(events).unwrap_or_default();
        let recovered = judge
            .assess(
                &format!("The reply recovers where the checkout-service work landed — that {DECISION}."),
                &format!(
                    "In an earlier session the team locked a decision: the checkout-service rewrite \
                     moves off the legacy billing module onto the new payments API. After an idle gap \
                     opened a fresh session, sam returned and asked \"remind me where we landed on the \
                     checkout-service work?\" The agent replied:\n\"{reply}\""
                ),
            )
            .await;

        vec![
            Verdict::metric_outcome(
                "the cold open re-surfaced the recently worked thread in its brief",
                resurfaced,
                "the cold session opened with a non-empty active-threads set",
                "the cold session's active-threads section was empty",
            ),
            verdict_from_judge_outcome(
                "the vague re-entry recovered the thread from the cold open",
                VerdictKind::Metric,
                recovered,
            ),
        ]
    }
}
