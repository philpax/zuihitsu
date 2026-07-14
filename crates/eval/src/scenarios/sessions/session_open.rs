//! The session-open checkpoint flush (issue #60). When a fresh conversation begins, the agent first
//! flushes the *other* live conversations' working state to memory, so the new conversation's brief
//! composes over the just-flushed state rather than racing it. This scenario drives a substantive
//! planning exchange in room A, then opens a genuinely new room B whose participant asks after that
//! thread — with **no** `CheckpointSweep` step, so only the session-open trigger can have synced the
//! state. It is the open-trigger counterpart to `checkpoint_syncs_parallel_rooms`, which disables the
//! open trigger and drives the timer sweep explicitly.

use std::sync::Arc;

use async_trait::async_trait;
use zuihitsu::{Event, EventPayload, PromptTemplateName};

use crate::{
    analysis,
    judge::Judge,
    package::{Bar, Category, ScenarioMeta, Verdict, VerdictKind, verdict_from_judge_outcome},
    scenario::Scenario,
    step::{EvalStep, Turn},
};

/// This module's scenarios.
pub fn scenarios() -> Vec<Arc<dyn Scenario>> {
    vec![Arc::new(SessionOpenSyncsParallelRooms)]
}

/// The substance threshold, tuned to the script's scale: room A's planning exchange clears it by a
/// wide margin, while room B's opening greeting stays under — so the open trigger flushes exactly room
/// A. The cooldown is waived by the open trigger regardless, but is set to zero for clarity.
const MIN_DELTA_CHARS: i64 = 400;

/// Room A: the release-planning channel where the decisions land.
const PLANNING: &str = "release-planning";
/// Room B: the support channel where the newly-arriving lead asks after the plan.
const SUPPORT: &str = "customer-support";

/// A support lead opens a brand-new channel and asks for the state of a launch that was just planned
/// in a parallel channel. Nothing swept the planning room explicitly — no `CheckpointSweep` — so if
/// the support room's reply carries the plan, the *session open itself* must have flushed the planning
/// room's working state to memory before the support room's brief composed. Rooms A and B share one
/// participant (Nadia), and room B introduces a lead (Theo) who was never in the planning room, so the
/// recap is legitimately his to receive.
pub struct SessionOpenSyncsParallelRooms;

#[async_trait]
impl Scenario for SessionOpenSyncsParallelRooms {
    fn meta(&self) -> ScenarioMeta {
        ScenarioMeta {
            name: "session_open_syncs_parallel_rooms".to_owned(),
            category: Category::Sessions,
            description: "Opening a new conversation first flushes the other live rooms' working \
                          state to memory, so the new room's brief composes over it. A support \
                          channel opens cold and asks after a launch just planned in a parallel \
                          channel — with no explicit checkpoint sweep, only the session-open flush \
                          can have carried the plan across. The recap must surface the planning \
                          room's decisions (metric), and a session-open flush turn must have landed \
                          with every session still open (metric)."
                .to_owned(),
            bar: Bar::Metric { threshold: 0.7 },
        }
    }

    fn needs_retrieval(&self) -> bool {
        true
    }

    fn steps(&self) -> Vec<EvalStep> {
        vec![
            // The open trigger is on (its default); the timer sweep is never driven in this script, so
            // any sync is the session-open flush's doing. A low substance threshold and zero cooldown
            // tune the gates to the script's scale.
            EvalStep::TuneCheckpoint {
                min_delta_chars: MIN_DELTA_CHARS,
                cooldown_seconds: 0,
                flush_on_open: true,
            },
            // Room A: the planning exchange — concrete decisions, an owner, and a date. The run
            // clock starts on Monday the 8th, so "Thursday the 11th" is the immediate Thursday: the
            // bare "Thursday" references later in the exchange resolve to the same day, keeping the
            // scenario's measurement on the cross-room sync rather than on temporal disambiguation
            // (a ship date on a later Thursday made half the recaps resolve "Thursday" to the wrong
            // week).
            Turn::new(
                "slack",
                PLANNING,
                "nadia",
                "Kicking off release planning for the 2.4 rollout. Proposal: we branch on Tuesday, \
                 run the canary on staging Wednesday, and ship to everyone Thursday the 11th. Priya, \
                 does that hold on your side?",
            )
            .with_present(&["nadia", "priya"])
            .into(),
            Turn::new(
                "slack",
                PLANNING,
                "priya",
                "Holds if we freeze the config today — doing that now. Two things to pin: I own the \
                 rollback plan, and we gate the Thursday ship on a green canary, no exceptions.",
            )
            .with_present(&["nadia", "priya"])
            .into(),
            Turn::new(
                "slack",
                PLANNING,
                "nadia",
                "Locked: branch Tuesday, canary Wednesday, ship Thursday the 11th, Priya owns \
                 rollback, and the ship gates on a green canary. I'll post the summary to the \
                 leads channel.",
            )
            .with_present(&["nadia", "priya"])
            .into(),
            // Room B opens cold — a genuinely new conversation. Its own greeting stays under the
            // substance threshold, so the open trigger flushes room A and not room B. Nadia bridges the
            // two rooms; Theo is new and was never in planning. No CheckpointSweep step precedes this:
            // only the session open can have synced the plan.
            Turn::new(
                "slack",
                SUPPORT,
                "theo",
                "Morning — I'm covering support escalations for the 2.4 launch and I've been out. \
                 Can someone catch me up on the rollout plan? I need the shape of the week before I \
                 answer customers.",
            )
            .with_present(&["theo", "nadia"])
            .into(),
            // Let the describe and index catch-ups the background daemons would provide settle, so the
            // flushed facts are described and searchable when room B's recap reads them back.
            EvalStep::Settle,
            Turn::new(
                "slack",
                SUPPORT,
                "nadia",
                "Theo just joined the support side and needs the rollout plan for 2.4 — can you give \
                 him the recap of what we locked in planning?",
            )
            .with_present(&["theo", "nadia"])
            .into(),
        ]
    }

    async fn assess(&self, events: &[Event], judge: &Judge) -> Vec<Verdict> {
        // The machinery held: a flush turn (its `produced_by` carries the Flush template) landed with
        // every session still open — a session-open flush by construction, since nothing idles or
        // compacts in this script and no explicit sweep runs.
        let flushed = events.iter().any(|event| {
            matches!(
                &event.payload,
                EventPayload::ConversationTurn { produced_by: Some(produced), .. }
                    if produced.template_name == PromptTemplateName::Flush
            )
        });
        let every_session_open = !events
            .iter()
            .any(|event| matches!(event.payload, EventPayload::SessionEnded { .. }));
        let synced_on_open = flushed && every_session_open;

        // Room B's replies are where the cross-room recall shows — the sync the open trigger buys.
        let room_b = analysis::agent_replies_in(events, "slack", SUPPORT).join("\n");
        let recall = judge
            .assess(
                "The reply recaps the release plan from the planning channel — it states that the \
                 2.4 rollout ships on Thursday the 11th (naming the weekday alone counts: the recap \
                 happens in the same week, so \"Thursday\" unambiguously refers to the 11th; only a \
                 different or missing ship day fails), and carries at least one of the supporting \
                 decisions (the Tuesday branch, the Wednesday staging canary, Priya owning the \
                 rollback plan, or the ship gating on a green canary).",
                &format!(
                    "In a parallel planning channel, the team locked a release plan: branch \
                     Tuesday, staging canary Wednesday, the 2.4 rollout shipping Thursday the 11th, \
                     Priya owning the rollback plan, and the ship gated on a green canary. In a \
                     separate support channel that had only just opened, Nadia then asked the agent \
                     to recap the plan for Theo, who was not in planning. The agent's replies in the \
                     support channel:\n\"{room_b}\""
                ),
            )
            .await;

        vec![
            verdict_from_judge_outcome(
                "recapped the planning-room decisions in the new support room",
                VerdictKind::Metric,
                recall,
            ),
            Verdict::metric_outcome(
                "a session-open flush landed with every session open",
                synced_on_open,
                "a Flush-provenance turn landed with no SessionEnded and no explicit sweep",
                "no session-open flush turn landed (or a session closed around it)",
            ),
        ]
    }
}
