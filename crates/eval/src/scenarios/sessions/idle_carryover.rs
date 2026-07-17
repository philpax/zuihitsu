//! Idle-reopen raw-transcript carryover (issue #86; spec §Compaction → raw-transcript carryover). A
//! session reopening after an idle gap carries a bounded tail of the previous session's recent
//! messages, so the agent resumes with the last few things actually *said* — not only the
//! memory-derived active threads a cold open re-surfaces (that analogue is `cold_open`). Without it the
//! reopened session opens blank on the transcript: it can recover a recorded thread but not an
//! ephemeral conversational detail that was said just before the gap and never filed to memory.
//!
//! The arc: a first session holds a light back-and-forth that settles a small, conversational detail
//! (a day and a prop to bring) — the kind of thing a warm continuation would have in front of it but an
//! agent would not necessarily memorialise. The room goes quiet past the idle gap, so the next message
//! opens a fresh session; the reopen should be *seeded* from the prior tail. The re-entry asks vaguely
//! to reconfirm the detail. The structural metric checks the reopened session carried a tail (the
//! mechanism fired); the judged metric checks the reply reconfirmed the detail from the tail rather
//! than asking the participant to restate it.

use std::sync::Arc;

use async_trait::async_trait;
use zuihitsu::{Event, TEST_PLATFORM};

use crate::{
    analysis,
    context::PAST_IDLE_GAP_MS,
    judge::Judge,
    package::{Bar, Category, ScenarioMeta, Verdict, VerdictKind, verdict_from_judge_outcome},
    scenario::Scenario,
    step::{EvalStep, Turn},
};

/// This module's scenarios.
pub fn scenarios() -> Vec<Arc<dyn Scenario>> {
    vec![Arc::new(IdleReopenCarriesThePriorTail)]
}

/// The conversational detail the tail should carry across the gap — a day and a prop, settled offhand
/// in the last exchange. Probed by meaning, not phrasing.
const DETAIL: &str = "the sketch review is on Thursday and rowan is bringing the good markers";

/// A room worked across an idle seam. In the first session rowan and the agent settle, casually, when
/// to do a sketch review and who brings what — an ephemeral detail, not a durable fact. The room then
/// falls quiet past the idle gap, so the next message opens a *fresh* session; issue #86 seeds it from
/// the prior tail, so the last thing said is still in the buffer. rowan returns and asks, vaguely, to
/// reconfirm. Without the carried tail the reopen opens blank on the transcript and has to ask rowan to
/// restate it; with it, the agent reconfirms from what it can still see.
pub struct IdleReopenCarriesThePriorTail;

#[async_trait]
impl Scenario for IdleReopenCarriesThePriorTail {
    fn meta(&self) -> ScenarioMeta {
        ScenarioMeta {
            name: "idle_reopen_carries_the_prior_tail".to_owned(),
            category: Category::Sessions,
            description: "A room settles a small conversational detail, then goes idle past the gap so \
                          the next message opens a fresh session. The reopen should be seeded from the \
                          prior session's raw-transcript tail (issue #86), so the vague re-entry \
                          reconfirms the detail from what was just said rather than asking for it \
                          again. A tracked quality rate, not a safety gate."
                .to_owned(),
            bar: Bar::Metric { threshold: 0.6 },
        }
    }

    fn steps(&self) -> Vec<EvalStep> {
        vec![
            // A first session: a light back-and-forth that settles a small, conversational detail.
            Turn::new(
                TEST_PLATFORM,
                "studio",
                "rowan",
                "Quick one before I disappear — can we do the sketch review this week? I don't want it \
                 sliding into next.",
            )
            .with_present(&["rowan"])
            .into(),
            Turn::new(
                TEST_PLATFORM,
                "studio",
                "rowan",
                "Thursday works for me. And I'll bring the good markers this time so we're not stuck \
                 with the dried-out ones.",
            )
            .with_present(&["rowan"])
            .into(),
            Turn::new(
                TEST_PLATFORM,
                "studio",
                "rowan",
                "Perfect, that's settled then. Running now — talk later.",
            )
            .with_present(&["rowan"])
            .into(),
            // Settle description and index catch-up so the state is durable before the seam.
            EvalStep::Settle,
            // The room goes quiet past the idle gap: the next message opens a fresh session, which
            // issue #86 seeds from the prior tail rather than opening blank on the transcript.
            EvalStep::Advance {
                millis: PAST_IDLE_GAP_MS,
            },
            // rowan returns and asks vaguely to reconfirm. The reconfirmation leans on the carried tail.
            Turn::new(
                TEST_PLATFORM,
                "studio",
                "rowan",
                "Back — remind me what we said for the sketch review?",
            )
            .with_present(&["rowan"])
            .into(),
        ]
    }

    async fn assess(&self, events: &[Event], judge: &Judge) -> Vec<Verdict> {
        // The mechanism, structurally: the last session opened (the one after the idle gap) should have
        // been seeded from the prior session's tail — its `seeded_from_turn` is set. Session 1 opened
        // fresh on first contact (`false`); the reopen carried a tail (`true`). Model-free, the direct
        // test of the fix.
        let seeds = analysis::session_seeds(events);
        let carried_tail = seeds.last() == Some(&true) && seeds.first() == Some(&false);

        // The behaviour, by meaning: the vague re-entry reconfirmed the settled detail rather than
        // asking rowan to restate it — the benefit the carried tail buys on an otherwise empty buffer.
        let reply = analysis::last_agent_reply(events).unwrap_or_default();
        let reconfirmed = judge
            .assess(
                &format!("The reply reconfirms the settled detail — that {DETAIL}."),
                &format!(
                    "In the session just before an idle gap, rowan and the agent settled a sketch \
                     review: Thursday, with rowan bringing the good markers. After the gap opened a \
                     fresh session, rowan returned and asked \"remind me what we said for the sketch \
                     review?\" The agent replied:\n\"{reply}\""
                ),
            )
            .await;

        vec![
            Verdict::metric_outcome(
                "the reopened session carried the prior session's raw-transcript tail",
                carried_tail,
                "the session after the idle gap was seeded from the prior tail",
                "the session after the idle gap opened without a carried tail",
            ),
            verdict_from_judge_outcome(
                "the vague re-entry reconfirmed the detail from the carried tail",
                VerdictKind::Metric,
                reconfirmed,
            ),
        ]
    }
}
