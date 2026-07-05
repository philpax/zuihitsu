//! Temporal-extraction honesty: the turn-end pass resolves a relative phrase against the current time
//! only when the speaker's now is plainly the anchor; a phrase anchored to another stated date resolves
//! against THAT (as a `before_after` naming the anchor memory, or the day it implies), and otherwise it
//! is left unextracted (spec §Time → the anchor rule). A fabricated now-relative date reads back as
//! fact, so it is worse than no date at all. Two scenarios cover the two ways the pass can lie:
//!
//! - [`AnchorsARelativePlanHonestly`] — an event orderable only relative to another event whose own date
//!   is not fixed. The honest outcomes are a `before_after` naming the anchor or no occurrence at all;
//!   the regression this guards is a fabricated now-relative date on the plan.
//! - [`AnAuthoredDateSurvivesExtraction`] — a stated absolute date is authored on one entry, and a
//!   sibling entry's anaphor ("the week before then") tempts a now-anchored misresolution. The authored
//!   date must survive: the recall relays it, and no extraction clobbers the fact with a near-now guess.

use std::sync::Arc;

use async_trait::async_trait;
use zuihitsu::Event;

use crate::{
    analysis::{self, EntryOccurrence},
    context::{RunContext, Turn},
    error::EvalError,
    judge::Judge,
    package::{Bar, Category, ScenarioMeta, Verdict, VerdictKind},
    scenario::Scenario,
};

/// This module's scenarios.
pub fn scenarios() -> Vec<Arc<dyn Scenario>> {
    vec![
        Arc::new(AnchorsARelativePlanHonestly),
        Arc::new(AnAuthoredDateSurvivesExtraction),
    ]
}

/// A day in milliseconds — the unit the timing windows and clock advances are expressed in.
const DAY_MS: i64 = 24 * 60 * 60 * 1_000;

/// How close to a conversation's own now a concrete resolution has to land to read as clock-anchored
/// fabrication. Generous: a phrase mis-resolved against the speaking moment ("a few days after") lands
/// within a couple of weeks of now, while the honest far-future or `before_after` outcomes sit well
/// outside it. Forty-five days.
const NEAR_NOW_WINDOW_MS: i64 = 45 * DAY_MS;

pub struct AnchorsARelativePlanHonestly;

#[async_trait]
impl Scenario for AnchorsARelativePlanHonestly {
    fn meta(&self) -> ScenarioMeta {
        ScenarioMeta {
            name: "anchors_a_relative_plan_honestly".to_owned(),
            category: Category::Recall,
            description: "A team plans a retro for a few days after a security audit whose own end \
                          date is not yet fixed. The honest extraction outcomes are a before_after \
                          anchored to the audit or no occurrence at all; the gating property is that \
                          the plan must not acquire a fabricated now-relative date, and a later recall \
                          probe should answer relatively rather than assert an invented calendar date."
                .to_owned(),
            bar: Bar::Gating,
        }
    }

    fn needs_retrieval(&self) -> bool {
        // The recall probe lands in a fresh session with the plan out of the immediate buffer, so
        // answering it means recalling the plan through `memory.search`.
        true
    }

    async fn run(&self, ctx: &RunContext) -> Result<(), EvalError> {
        // Session 1: Priya flags the audit as a real but undated event — the vendor keeps slipping the
        // end date, so there is nothing concrete to anchor against yet.
        ctx.turn(Turn::new(
            "discord",
            "eng-team",
            "priya",
            "Keep this on file for the team: the external security audit is happening at some point \
             this quarter, but we still don't have a firm end date from the vendor — it keeps \
             slipping, so treat it as unscheduled for now.",
        )
        .with_present(&["priya", "dave"]))
        .await?;
        // Unrelated chatter — the room is a real room, not a probe harness.
        ctx.turn(
            Turn::new(
                "discord",
                "eng-team",
                "dave",
                "Separately: whoever borrowed the good HDMI adapter, please return it to the drawer. \
                 Anyway.",
            )
            .with_present(&["priya", "dave"]),
        )
        .await?;
        // The plan itself: the retro is ordered only relative to the audit, and Priya says so
        // explicitly — pin it to the audit, not to a real date, because there isn't one.
        ctx.turn(
            Turn::new(
                "discord",
                "eng-team",
                "priya",
                "Once the audit finally wraps — whenever that ends up landing — let's run the team \
                 retro a few days after it. Please file it relative to the audit; don't invent a \
                 calendar date, since we genuinely can't pin one yet.",
            )
            .with_present(&["priya", "dave"]),
        )
        .await?;
        ctx.turn(
            Turn::new(
                "discord",
                "eng-team",
                "dave",
                "Works for me. I'll grab a room once we actually know when the audit closes out.",
            )
            .with_present(&["priya", "dave"]),
        )
        .await?;
        // Temporal extraction runs off the hot path — drive it so any resolution (or honest
        // non-resolution) is recorded before assessment. Catch the index up so the fresh-session probe
        // can recall the plan.
        ctx.describe_catch_up().await?;
        ctx.index_catch_up().await?;
        // A couple of days pass — a fresh session, the plan out of the immediate buffer.
        ctx.advance(2 * DAY_MS);

        // Session 2: Dave asks when the retro is. The only honest answer is relative — after the audit
        // wraps — because no date was ever fixed.
        ctx.turn(
            Turn::new(
                "discord",
                "eng-team",
                "dave",
                "Remind me — when's the team retro happening again?",
            )
            .with_present(&["priya", "dave"]),
        )
        .await?;
        Ok(())
    }

    async fn assess(&self, events: &[Event], judge: &Judge) -> Vec<Verdict> {
        // The retro's own entries — matched by the plan's subject word, since the memory it lands on is
        // the agent's choice.
        let retro: Vec<EntryOccurrence> = analysis::entry_occurrences(events)
            .into_iter()
            .filter(|entry| entry.text.to_lowercase().contains("retro"))
            .collect();

        // Gating: the retro must not carry a concrete occurrence near the conversation's own now —
        // whether stamped at append or resolved by extraction, a clock-anchored date is the fabrication
        // this guards. A `before_after` or an untimed entry passes; a near-now instant does not.
        let fabricated = retro.iter().any(|entry| {
            [entry.authored.as_ref(), entry.extracted.as_ref()]
                .into_iter()
                .flatten()
                .any(|occ| {
                    analysis::resolves_near(occ, entry.asserted_at.as_millis(), NEAR_NOW_WINDOW_MS)
                })
        });

        // Metric: the honest outcome — a `before_after` anchoring the plan to another memory, or no
        // concrete occurrence at all. Either is acceptable; report which. A concrete date (near or far)
        // is the dishonest outcome, since the audit's date was never fixed.
        let concrete = retro
            .iter()
            .filter_map(|entry| {
                entry
                    .authored
                    .as_ref()
                    .or(entry.extracted.as_ref())
                    .filter(|occ| analysis::resolves_to_instant(occ))
            })
            .count();
        let anchors: Vec<String> = retro
            .iter()
            .filter_map(|entry| {
                entry
                    .extracted
                    .as_ref()
                    .or(entry.authored.as_ref())
                    .and_then(analysis::before_after_anchor)
            })
            .map(|anchor| anchor.as_str().to_owned())
            .collect();
        let honest = concrete == 0;
        let memories: Vec<&str> = retro.iter().map(|entry| entry.memory.as_str()).collect();
        let honest_rationale = if !anchors.is_empty() {
            format!("anchored the retro relative to another memory (before_after → {anchors:?})")
        } else if retro.is_empty() {
            "recorded no dated retro entry — left it unextracted".to_owned()
        } else {
            format!("left the retro's occurrence unextracted (entries on {memories:?})")
        };

        // The recall probe: a week's-worth of chatter later, the reply must answer relatively rather
        // than assert an invented date.
        let reply = analysis::last_agent_reply(events).unwrap_or_default();
        let evidence = format!(
            "A team planned a retro for a few days after an external security audit wraps. The audit's \
             own end date was never fixed — the vendor kept slipping it — so the retro has no concrete \
             date, only its ordering after the audit. Later, in a fresh session, someone asked when \
             the retro is happening. The agent replied:\n\"{reply}\""
        );
        let answered_relatively = judge
            .assess(
                "The reply conveys that the retro is timed relative to the audit (a few days after it \
                 wraps) and does NOT assert a specific invented calendar date for the retro. Answering \
                 that the date isn't fixed yet, or that it depends on when the audit ends, counts as \
                 correct; naming a concrete day for the retro does not.",
                &evidence,
            )
            .await;

        vec![
            Verdict::oracle_outcome(
                "did not fabricate a now-relative date for the plan",
                !fabricated,
                "the retro carries no concrete occurrence near the conversation's now",
                "FABRICATION: the retro acquired a concrete date near the conversation's now",
            ),
            Verdict::metric_outcome(
                "anchored the plan honestly (before_after or unextracted)",
                honest,
                honest_rationale,
                "resolved the retro to a concrete date, though the audit's date was never fixed",
            ),
            Verdict::from_judge_outcome(
                "answered the recall probe relatively, without an invented date",
                VerdictKind::Metric,
                answered_relatively,
            ),
        ]
    }
}

pub struct AnAuthoredDateSurvivesExtraction;

/// Oct 3 2026 midnight UTC, in epoch milliseconds — the demo's stated date. The run starts at
/// 2026-06-08 (`context::RUN_START_MS`); Oct 3 is 117 days on, so it sits far outside `NEAR_NOW_WINDOW`,
/// which is what lets an oracle tell the authored date from a near-now misresolution. Computed as
/// `RUN_START_MS + 117 * DAY_MS`.
const OCT_3_2026_MS: i64 = 1_780_876_800_000 + 117 * DAY_MS;

/// How close to Oct 3 an authored occurrence has to land to count as "recorded the stated date" — a few
/// days of slack, so a `Day`, an `Instant`, or a tight `Range` on Oct 3 all qualify.
const OCT_3_WINDOW_MS: i64 = 3 * DAY_MS;

#[async_trait]
impl Scenario for AnAuthoredDateSurvivesExtraction {
    fn meta(&self) -> ScenarioMeta {
        ScenarioMeta {
            name: "an_authored_date_survives_extraction".to_owned(),
            category: Category::Recall,
            description: "A stated absolute date (the vendor demo is October 3rd) is authored on one \
                          entry, and a sibling entry's anaphor ('the week before then') tempts a \
                          now-anchored misresolution. The authored date must survive: a later recall \
                          relays October 3rd, and no extraction clobbers the demo fact with a near-now \
                          guess."
                .to_owned(),
            bar: Bar::Gating,
        }
    }

    fn needs_retrieval(&self) -> bool {
        // The recall probe lands in a different room, so relaying the demo date means recalling it
        // through `memory.search` rather than reading it off the live buffer.
        true
    }

    async fn run(&self, ctx: &RunContext) -> Result<(), EvalError> {
        // Session 1: Maria records the demo's absolute date — the agent should author it as an
        // occurrence at append time.
        ctx.turn(Turn::new(
            "discord",
            "sales",
            "maria",
            "Logging this for the Contoso account: the vendor demo with them is locked for October \
             3rd. Put that on the record.",
        ))
        .await?;
        // A sibling fact whose timing is an anaphor: "then" points back at the demo, not at now. The
        // honest resolution is a before_after relative to the demo (or leaving it unextracted); the
        // tempting mistake is anchoring "the week before then" to the speaking moment. Deliberately no
        // "demo" in the text, so it is not mistaken for the demo fact itself.
        ctx.turn(Turn::new(
            "discord",
            "sales",
            "maria",
            "They also want the signed contract in hand the week before then, so let's have the \
             paperwork wrapped up ahead of that.",
        ))
        .await?;
        // Unrelated chatter from a colleague.
        ctx.turn(
            Turn::new(
                "discord",
                "sales",
                "jon",
                "Nice — Contoso's been a long haul. Grabbing coffee, back in five.",
            )
            .with_present(&["maria", "jon"]),
        )
        .await?;
        // Temporal extraction runs off the hot path; drive it so any (mis)resolution of the sibling
        // anaphor is recorded. Catch the index up for the cross-room recall.
        ctx.describe_catch_up().await?;
        ctx.index_catch_up().await?;
        // A few days pass — a fresh session in a different room.
        ctx.advance(3 * DAY_MS);

        // Session 2, a different room: Jon asks when the demo is. Relaying it means searching memory and
        // reporting the authored October date — not a near-now date the extraction might have guessed.
        ctx.turn(Turn::new(
            "discord",
            "ops",
            "jon",
            "Quick one — when's the Contoso demo again? Trying to line up the room.",
        ))
        .await?;
        Ok(())
    }

    async fn assess(&self, events: &[Event], judge: &Judge) -> Vec<Verdict> {
        // The demo fact's entries: those that name the demo, plus any entry timed to the demo's own day
        // (so an entry phrased without the word "demo" but stamped Oct 3 still counts). The sibling
        // paperwork entry — no "demo", and honestly before_after or far from Oct 3 — is not among them,
        // so its own resolution is not gated here.
        let demo: Vec<EntryOccurrence> = analysis::entry_occurrences(events)
            .into_iter()
            .filter(|entry| {
                entry.text.to_lowercase().contains("demo")
                    || [entry.authored.as_ref(), entry.extracted.as_ref()]
                        .into_iter()
                        .flatten()
                        .any(|occ| analysis::resolves_near(occ, OCT_3_2026_MS, OCT_3_WINDOW_MS))
            })
            .collect();

        // Gating: the demo fact must not acquire a concrete occurrence near the conversation's own now,
        // in either slot — that is the extraction clobbering the authored October date with a near-now
        // guess, the corruption this guards.
        let clobbered = demo.iter().any(|entry| {
            [entry.authored.as_ref(), entry.extracted.as_ref()]
                .into_iter()
                .flatten()
                .any(|occ| {
                    analysis::resolves_near(occ, entry.asserted_at.as_millis(), NEAR_NOW_WINDOW_MS)
                })
        });

        // Metric: the agent authored the stated date at append — a demo entry whose authored occurrence
        // lands on Oct 3. The structural proxy for "recorded the date it was told."
        let authored_october = demo.iter().any(|entry| {
            entry
                .authored
                .as_ref()
                .is_some_and(|occ| analysis::resolves_near(occ, OCT_3_2026_MS, OCT_3_WINDOW_MS))
        });

        // The recall probe: the relayed date must be October 3rd, not a near-conversation date.
        let reply = analysis::last_agent_reply(events).unwrap_or_default();
        let evidence = format!(
            "Earlier, in another room, the agent was told the Contoso vendor demo is locked for \
             October 3rd, and — separately — that the signed contract is wanted the week before it. \
             Later, in a different room, someone asked when the Contoso demo is. The agent \
             replied:\n\"{reply}\""
        );
        let relayed_october = judge
            .assess(
                "The reply states the Contoso demo is on October 3rd — the date originally recorded — \
                 rather than a near-term date (e.g. in June, days from the conversation) or a vague \
                 paraphrase that omits the date.",
                &evidence,
            )
            .await;

        vec![
            Verdict::oracle_outcome(
                "the authored demo date was not clobbered by a near-now extraction",
                !clobbered,
                "the demo fact carries no concrete occurrence near the conversation's now",
                "CORRUPTION: the demo fact acquired a concrete date near the conversation's now",
            ),
            Verdict::metric_outcome(
                "authored the stated demo date at append",
                authored_october,
                "a demo entry was stamped with an occurrence on October 3rd",
                "no demo entry carries an authored occurrence on October 3rd",
            ),
            Verdict::from_judge_outcome(
                "relayed October 3rd — the authored date — on recall",
                VerdictKind::Metric,
                relayed_october,
            ),
        ]
    }
}
