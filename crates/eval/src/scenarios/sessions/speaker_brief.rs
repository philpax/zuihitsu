//! The initiating speaker's guaranteed brief block (issue #85, spec §Contextual briefs → present-set
//! cap). The person whose inbound message opens a session is the one the agent is about to answer, so
//! the composed brief guarantees them a full `## <handle>` participant block — rendered ahead of the
//! other present participants and exempt from both the present-set cap and the char-budget drop.
//! Before the fix, present participants were ranked purely by memory recency: if the speaker's memory
//! had been touched less recently than another present participant's, the speaker could be collapsed to
//! a bare `- <handle> (present)` name-only line, and the agent replied to someone it had not been
//! briefed on.
//!
//! The arc reproduces that shape without leaning on budget starvation. In a first session two
//! colleagues share a room: the non-speaker (rowan) speaks last and richest, so its memory is the more
//! recently touched and would win the recency ranking; the speaker (wren) is recorded earlier and more
//! thinly. The room then goes idle past the gap, and wren — the recency loser — sends the message that
//! opens a fresh session, so wren is the initiating speaker the guarantee must protect even though rowan
//! outranks it. The oracles are structural: the speaker is threaded onto `SessionStarted.initiators`
//! end to end, and the composed brief carries a full `## <handle>` block for the speaker rather than a
//! name-only line. A judged metric adds that the reply actually drew on what the agent knew about wren.

use std::sync::Arc;

use async_trait::async_trait;
use zuihitsu::{Event, EventPayload, MemoryId, MemoryName, PersonId, Seq, TEST_PLATFORM};

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
    vec![Arc::new(SpeakerGuaranteedAFullBriefBlock)]
}

/// The room both colleagues share and the fresh session opens in.
const ROOM: &str = "payments-sync";
/// The initiating speaker — recorded earlier and more thinly, so it loses the recency ranking, yet
/// opens the fresh session and so must be guaranteed a full brief block.
const SPEAKER: &str = "wren";
/// The non-speaker who speaks last and richest in the first session, so its memory is the more recently
/// touched and would win the recency ranking that the speaker guarantee overrides.
const RECENCY_WINNER: &str = "rowan";

/// Two colleagues work a room; the one with the less-recently-touched memory later opens a fresh session
/// and is the person the agent must answer. The brief must give that initiating speaker a full block
/// even though the other present participant outranks it on recency — the person the agent is replying
/// to is never reduced to a name-only line (issue #85).
pub struct SpeakerGuaranteedAFullBriefBlock;

#[async_trait]
impl Scenario for SpeakerGuaranteedAFullBriefBlock {
    fn meta(&self) -> ScenarioMeta {
        ScenarioMeta {
            name: "speaker_guaranteed_full_brief_block".to_owned(),
            category: Category::Sessions,
            description: "The person whose message opens a session is guaranteed a full participant \
                          block in the composed brief, even when another present participant has \
                          more-recently-touched memory and outranks them on recency. A colleague with \
                          the richer, fresher memory speaks last in a first session; after an idle gap \
                          the thinner-memoried colleague opens a fresh session, so that speaker must \
                          still be briefed in full rather than collapsed to a name-only present line. \
                          The speaker is recorded on the session's initiators (gating), and the brief \
                          renders a full block for them (gating); the reply draws on what the agent \
                          knew about them (metric)."
                .to_owned(),
            // The two structural verdicts are deterministic machinery invariants, not model
            // judgements: the initiators recording and the full-block guarantee are pure functions of
            // the session-open path, so under correct code they hold on every run and a regression
            // drives them to zero. That makes this a must-not-regress safety property, so the bar
            // gates at 1.0 — one slip fails the harness. The judged reply metric rides as a
            // non-gating `Metric` verdict, so a model miss lowers the rate without failing the gate.
            bar: Bar::gating(),
        }
    }

    fn steps(&self) -> Vec<EvalStep> {
        vec![
            // First session. The speaker (wren) is introduced first, so its memory is the earlier —
            // and, after the following turns, the less recently touched — of the two.
            Turn::new(
                TEST_PLATFORM,
                ROOM,
                SPEAKER,
                "Hi all — wren here, just moved over from the risk team to lead payments. My focus \
                 this quarter is the fraud-detection rework, and I'm running it out of the Berlin \
                 office.",
            )
            .with_present(&[SPEAKER, RECENCY_WINNER])
            .into(),
            // The non-speaker (rowan) then speaks last and richest, so its memory is touched most
            // recently and carries the most facts — the recency winner the guarantee must override.
            Turn::new(
                TEST_PLATFORM,
                ROOM,
                RECENCY_WINNER,
                "And rowan — I run the mobile guild. I'm just back from parental leave, and my big \
                 push right now is landing offline-sync before the March cutoff.",
            )
            .with_present(&[SPEAKER, RECENCY_WINNER])
            .into(),
            Turn::new(
                TEST_PLATFORM,
                ROOM,
                RECENCY_WINNER,
                "One more for the record: I've picked up the release-train schedule this half, and \
                 I'm mentoring the two new grads on the mobile side while they ramp.",
            )
            .with_present(&[SPEAKER, RECENCY_WINNER])
            .into(),
            // Settle the describe and index catch-ups so both colleagues' facts are synthesized and
            // durable before the seam.
            EvalStep::Settle,
            // The room goes quiet past the idle gap: the next message opens a fresh, cold session.
            EvalStep::Advance {
                millis: PAST_IDLE_GAP_MS,
            },
            // The speaker returns and opens the fresh session with rowan present. wren is the recency
            // loser but the initiating speaker, so the brief for this session must brief wren in full.
            Turn::new(
                TEST_PLATFORM,
                ROOM,
                SPEAKER,
                "Morning — I'm about to onboard a contractor onto my area today and want to frame the \
                 scope consistently. Can you give me a two-line summary of what I'm currently leading, \
                 so I don't undersell it?",
            )
            .with_present(&[SPEAKER, RECENCY_WINNER])
            .into(),
        ]
    }

    async fn assess(&self, events: &[Event], judge: &Judge) -> Vec<Verdict> {
        // The speaker's auto-created stub, minted on first contact in the first session. `MemoryCreated`
        // records the original `person/<id>@<platform>` handle, so this resolves the speaker's memory id
        // regardless of any later rename.
        let speaker_handle = MemoryName::from(PersonId::new(TEST_PLATFORM, SPEAKER));
        let speaker_id = analysis::memory_id_named(events, speaker_handle.as_str());

        // The session under test is the latest one the speaker opened: the fresh cold session on wren's
        // return past the idle gap. Targeting the latest `SessionStarted` whose `initiators` name the
        // speaker (rather than the last `SessionStarted` outright) keeps the oracle robust to an
        // agent-scheduled wake firing during the advanced gap, which would open an intervening
        // agent-initiated session with empty initiators. If the guarantee regressed, no session names
        // the speaker as an initiator and both checks below fail — the intended failure.
        let session = speaker_id.as_ref().and_then(|id| {
            events.iter().rev().find_map(|event| match &event.payload {
                EventPayload::SessionStarted {
                    brief, initiators, ..
                } if initiators.contains(id) => Some((event.seq, brief.clone())),
                _ => None,
            })
        });

        // Check 1: the speaker was threaded onto `SessionStarted.initiators` end to end. A future
        // refactor that forgets to pass the speakers down to the recorder leaves the field empty, so no
        // session names the speaker and this fails — the threading's regression guard.
        let threaded = session.is_some();

        // Check 2: the composed brief gives the speaker a full `## <handle>` participant block, not a
        // name-only `- <handle> (present)` line — the composition guarantee. The header is matched
        // against the speaker's name as the brief rendered it (resolved as of the session-open seq,
        // folding any rename), so the check reads the exact string the composer wrote.
        let briefed_in_full = match (&speaker_id, &session) {
            (Some(id), Some((seq, brief))) => memory_name_at(events, *id, *seq)
                .is_some_and(|name| brief.contains(&format!("## {name}"))),
            _ => false,
        };

        // The reply, by meaning: the agent answered from what it knew about the speaker rather than
        // asking wren to reintroduce themselves — the payoff of being briefed in full.
        let reply = analysis::last_agent_reply(events).unwrap_or_default();
        let drew_on_speaker = judge
            .assess(
                "The reply draws on at least one recorded fact about the person who asked — that they \
                 lead payments, own the fraud-detection rework this quarter, moved over from the risk \
                 team, or are based in the Berlin office — rather than asking them to say who they are \
                 or what they work on.",
                &format!(
                    "In an earlier session wren introduced themselves: they moved from the risk team \
                     to lead payments, own the fraud-detection rework this quarter, and run it out of \
                     the Berlin office. After an idle gap opened a fresh session, wren returned and \
                     asked for a two-line summary of what they are currently leading, to frame it for \
                     a contractor they are onboarding. The agent replied:\n\"{reply}\""
                ),
            )
            .await;

        vec![
            Verdict::oracle_outcome(
                "recorded the initiating speaker on the session it opened",
                threaded,
                "the fresh session's initiators carry the speaker's memory id",
                "the fresh session recorded no initiator, or not the speaker",
            ),
            Verdict::oracle_outcome(
                "briefed the initiating speaker with a full participant block",
                briefed_in_full,
                "the composed brief renders a full `## <handle>` block for the speaker",
                "the speaker was collapsed to a name-only present line rather than a full block",
            ),
            verdict_from_judge_outcome(
                "the reply drew on what the agent knew about the speaker",
                VerdictKind::Metric,
                drew_on_speaker,
            ),
        ]
    }
}

/// The name memory `target` bore as of sequence `up_to`, folding its creation and any later renames up
/// to that point — the exact `## <handle>` the brief composed at the session open would have written.
/// Resolving the header this way keeps the full-block check robust to the agent renaming the speaker's
/// stub, since it reads whatever name the memory held when the brief was frozen.
fn memory_name_at(events: &[Event], target: MemoryId, up_to: Seq) -> Option<String> {
    let mut name = None;
    for event in events {
        if event.seq > up_to {
            continue;
        }
        match &event.payload {
            EventPayload::MemoryCreated { id, name: created } if *id == target => {
                name = Some(created.as_str().to_owned());
            }
            EventPayload::MemoryRenamed { id, new_name, .. } if *id == target => {
                name = Some(new_name.as_str().to_owned());
            }
            _ => {}
        }
    }
    name
}
