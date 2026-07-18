//! Per-conversation turn supersession (issue #83): a correction that lands mid-reply. When a batch
//! arrives while the agent is still generating an answer, the platform cancels the in-flight generation
//! and answers once with everything in context — a person mid-compose who receives another message
//! re-reads and writes one reply, rather than firing off two.
//!
//! The behavioural property this scenario measures: a burst interrupted mid-generation yields one reply
//! that coheres over the whole burst. A few turns settle Saturday's dinner logistics (who is coming, that
//! one guest is vegetarian, the table booked for 7pm), the supersession window is widened so a real
//! turn's generation lands inside it, and then the burst: the coordinator asks for a synthesis-heavy
//! summary they are about to paste into a group message — a long, interruptible generation — and, while
//! the agent is composing it, corrects the time to 8:30. The successor must answer once, giving 8:30 as
//! the current time, reading as a single clean summary with no meta-acknowledgement of the interruption.
//!
//! The oracles emit conditionally so the timing race stays out of the behaviour rates. Two hold on every
//! run: both burst messages are durably recorded as participant turns (the platform contract), and the
//! interrupt landing mid-generation is reported as a metric (the landed-rate, so a package where the race
//! always missed reads as hollow rather than silently green). The rest apply only when the interrupt
//! actually landed: exactly one agent turn answers the burst (a superseded turn that still posted is the
//! double-response bug), and — judged — the reply gives 8:30 not 7pm as current and reads as one coherent
//! answer. A brief acknowledgement of the corrected time is fine — a person corrected mid-compose says
//! "got it, 8:30" and moves on; what fails coherence is narrating the interruption itself, or a
//! two-draft reply. The bar gates on the structural oracles; the judged criteria stay metrics.

use std::sync::Arc;

use async_trait::async_trait;
use zuihitsu::{Event, TEST_PLATFORM};

use crate::{
    analysis,
    judge::Judge,
    package::{Bar, Category, ScenarioMeta, Verdict, VerdictKind, verdict_from_judge_outcome},
    scenario::Scenario,
    step::{BurstMessage, EvalStep, InterruptedTurn, Turn},
};

/// This module's scenarios.
pub fn scenarios() -> Vec<Arc<dyn Scenario>> {
    vec![Arc::new(ACorrectionLandsMidReply)]
}

/// The planning room the coordinator and the agent settle Saturday's dinner in.
const ROOM: &str = "saturday-dinner";
/// The coordinator who settles the plan and then bursts the summary-then-correction. Both burst messages
/// come from them: a person mid-compose correcting their own request.
const COORDINATOR: &str = "noor";

/// The burst's first message: a synthesis-heavy ask the coordinator is about to paste into a group chat.
/// Long and open-ended on purpose — a summary of everything settled is a slow generation, so the
/// correction has time to land while the agent is still composing it. Held as a constant so the oracle
/// that checks it was recorded as a participant turn matches the exact text delivered.
const FIRST_ASK: &str = "Okay, I'm about to fire this off to the group chat — can you pull together \
    everything we've settled for Saturday into one clean summary I can paste? Who's coming, where it \
    is, what time, and please flag the veg thing so nobody books us somewhere that can't handle it.";

/// The burst's interrupt: the coordinator corrects the time before the summary lands. It arrives while
/// the first ask is still generating, so the platform supersedes that generation and the successor
/// answers with the corrected time in context.
const CORRECTION: &str = "Wait — scratch the 7pm. Priya's train gets in at 8, so I moved the booking to \
    8:30. Same place, Rosalind's.";

/// A correction that lands while the agent is mid-reply: the agent must answer once, over the merged
/// burst, with the corrected time — not fire off a summary built on the stale 7pm and then a second
/// message walking it back (issue #83).
pub struct ACorrectionLandsMidReply;

#[async_trait]
impl Scenario for ACorrectionLandsMidReply {
    fn meta(&self) -> ScenarioMeta {
        ScenarioMeta {
            name: "a_correction_lands_mid_reply".to_owned(),
            category: Category::Sessions,
            description: "A coordinator settles Saturday's dinner (who is coming, one guest vegetarian, \
                          a table booked for 7pm), then asks for a paste-ready summary and, while the \
                          agent is composing it, corrects the time to 8:30. The in-flight generation is \
                          superseded and one reply answers the merged burst. Both burst messages are \
                          recorded as participant turns (gating); the interrupt landing mid-generation is \
                          reported (metric); when it landed, exactly one agent turn answers (gating), and \
                          — judged — the reply gives 8:30 not 7pm and reads as one coherent summary \
                          (metrics)."
                .to_owned(),
            // Gates on the structural oracles — both messages recorded, and exactly one agent turn
            // answering a landed interrupt. The judged criteria (corrected time, coherence) stay
            // metrics: language judgements carry an error band a one-slip gate would misfire on. The
            // landed-rate itself is a metric too — a missed race is timing, not agent misbehaviour —
            // but if it sinks, lengthen `FIRST_ASK` so the generation is slower and easier to
            // interrupt, rather than loosening the oracles.
            bar: Bar::gating(),
        }
    }

    fn steps(&self) -> Vec<EvalStep> {
        vec![
            // Settle the plan over a few natural turns, all in one warm session so the facts sit in the
            // live buffer the burst reads from.
            Turn::new(
                TEST_PLATFORM,
                ROOM,
                COORDINATOR,
                "Hey Kestrel — helping me pull Saturday's dinner together. It's me, Priya, and Theo so \
                 far, and Marc might join if he's back from Leeds in time.",
            )
            .into(),
            Turn::new(
                TEST_PLATFORM,
                ROOM,
                COORDINATOR,
                "One thing to hold onto: Priya's vegetarian, and she's been burned before by places \
                 that just do a token salad, so wherever we land needs a proper veg menu.",
            )
            .into(),
            Turn::new(
                TEST_PLATFORM,
                ROOM,
                COORDINATOR,
                "Booked it — table for four at Rosalind's, 7pm Saturday. If Marc confirms I'll call and \
                 bump it to five.",
            )
            .into(),
            Turn::new(
                TEST_PLATFORM,
                ROOM,
                COORDINATOR,
                "Oh, and Theo asked if it's walkable from the station — it is, about ten minutes, so no \
                 need to sort parking.",
            )
            .into(),
            // Widen the supersession window so a real generation lands inside it: a synthesis turn can
            // easily outlast the 60s default, and a queued (rather than superseding) interrupt would
            // never exercise the behaviour under test.
            EvalStep::TuneSupersession { window_seconds: 600 },
            // The burst: the summary ask, interrupted mid-generation by the time correction.
            InterruptedTurn::new(
                TEST_PLATFORM,
                ROOM,
                BurstMessage::new(COORDINATOR, FIRST_ASK),
                BurstMessage::new(COORDINATOR, CORRECTION),
            )
            .into(),
        ]
    }

    async fn assess(&self, events: &[Event], judge: &Judge) -> Vec<Verdict> {
        // Unconditional gating: both burst messages landed durably as participant turns, whichever way
        // the race fell. The platform records every inbound before answering — even the message whose
        // turn was superseded keeps its participant turn.
        let first_recorded = analysis::participant_turn_recorded(events, FIRST_ASK);
        let correction_recorded = analysis::participant_turn_recorded(events, CORRECTION);
        let both_recorded = first_recorded && correction_recorded;

        // Unconditional metric: did the interrupt land mid-generation? A `ModelCallAborted` with the
        // supersession cause is the durable proof. This is the landed-rate — reported so a package where
        // the race always missed is visibly hollow, not silently passing on the conditional oracles it
        // never got to test.
        let landed = analysis::superseded_abort_present(events);

        let mut verdicts = vec![
            Verdict::oracle_outcome(
                "recorded both burst messages as participant turns",
                both_recorded,
                "both the summary ask and the correction are durable participant turns",
                "a burst message is missing from the log as a participant turn",
            ),
            Verdict::metric_outcome(
                "the interrupt landed mid-generation",
                landed,
                "a superseded-cause model-call abort marks the cancelled generation",
                "the first turn finished before the interrupt arrived — the race missed this run",
            ),
        ];

        // The conditional oracles apply only when the interrupt actually superseded a generation: a run
        // where the race missed never exercised the merged-answer behaviour, so scoring it would measure
        // timing, not conduct. Emitting nothing here shrinks the criterion's denominator rather than
        // recording a spurious pass or fail.
        if landed {
            // Conditional gating: exactly one agent turn answers the burst. A superseded turn records no
            // agent turn, so the successor's single reply is the only one; two would be the
            // double-response bug. Anchored at the summary ask's inbound seq, so the settling turns'
            // replies are not counted.
            let answers = analysis::participant_turn_seq(events, FIRST_ASK)
                .map(|seq| analysis::agent_replies_after(events, seq))
                .unwrap_or(0);
            verdicts.push(Verdict::oracle_outcome(
                "answered the burst exactly once",
                answers == 1,
                "one agent turn answers the merged burst",
                "the burst drew more than one agent reply (or none) — the double-response bug",
            ));

            // The reply the successor posted — the last responding agent turn.
            let reply = analysis::last_agent_reply(events).unwrap_or_default();

            let current_time = judge
                .assess(
                    "The reply presents 8:30 (equivalently, half past eight) as the dinner time to tell \
                     people. It does not present 7pm as the current time, other than to note the change \
                     from it; if it mentions 7pm at all, it is clearly the old, corrected time.",
                    &format!(
                        "A coordinator settled a Saturday dinner and booked a table for 7pm. They then \
                         asked the agent for a summary to paste into a group chat and, before it landed, \
                         corrected the time: the booking moved to 8:30 at the same place. The agent \
                         replied:\n\"{reply}\""
                    ),
                )
                .await;

            let coherent = judge
                .assess(
                    "The reply reads as one coherent answer to the merged request, stating the settled \
                     plan as it now stands. A brief acknowledgement of the corrected time (for example \
                     \"updated to 8:30\" or \"got it\") before or within the summary is acceptable. \
                     What fails is narrating the interruption itself — apologising for or referring to \
                     being cut off mid-message, restarting or abandoning an earlier draft, or \
                     presenting two versions of the summary.",
                    &format!(
                        "The agent was asked for a paste-ready summary of a Saturday dinner and, while \
                         composing it, received a correction moving the time to 8:30. Its reply was:\n\
                         \"{reply}\""
                    ),
                )
                .await;

            verdicts.push(verdict_from_judge_outcome(
                "the reply gives the corrected time as current",
                VerdictKind::Metric,
                current_time,
            ));
            verdicts.push(verdict_from_judge_outcome(
                "the reply reads as one coherent answer",
                VerdictKind::Metric,
                coherent,
            ));
        }

        verdicts
    }
}
