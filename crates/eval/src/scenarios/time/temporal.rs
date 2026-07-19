//! Temporal-extraction honesty: the turn-end pass resolves a relative phrase against the current time
//! only when the speaker's now is plainly the anchor; a phrase anchored to another stated date resolves
//! against THAT (as a `before_after` naming the anchor memory, or the day it implies), and otherwise it
//! is left unextracted (spec §Time → the anchor rule). A fabricated now-relative date reads back as
//! fact, so it is worse than no date at all. Three scenarios cover the ways the pass can lie:
//!
//! - [`AnchorsARelativePlanHonestly`] — an event orderable only relative to another event whose own date
//!   is not fixed. The honest outcomes are a `before_after` naming the anchor or no occurrence at all;
//!   the regression this guards is a fabricated now-relative date on the plan.
//! - [`AnAuthoredDateSurvivesExtraction`] — a stated absolute date is authored on one entry, and a
//!   sibling entry's anaphor ("the week before then") tempts a now-anchored misresolution. The authored
//!   date must survive: the recall relays it, and no extraction clobbers the fact with a near-now guess.
//! - [`AnInspirationsDateStaysOffTheSubject`] — a statement mentions a genuine, pinnable historical
//!   date that describes a *different* referent than the subject (a nickname's namesake). The subject's
//!   own facts are timeless, so the honest outcome is no occurrence at all; the regression this guards
//!   is the namesake's date stamped onto the subject as if the subject's fact happened then.

use std::sync::Arc;

use async_trait::async_trait;
use zuihitsu::{Event, TEST_PLATFORM, Timestamp};

use crate::{
    analysis::{self, EntryOccurrence},
    context::{MILLIS_PER_DAY, civil_timestamp},
    judge::Judge,
    package::{Bar, Category, ScenarioMeta, Verdict, VerdictKind, verdict_from_judge_outcome},
    scenario::Scenario,
    step::{EvalStep, Turn},
};

/// This module's scenarios.
pub fn scenarios() -> Vec<Arc<dyn Scenario>> {
    vec![
        Arc::new(AnchorsARelativePlanHonestly),
        Arc::new(AnAuthoredDateSurvivesExtraction),
        Arc::new(AnInspirationsDateStaysOffTheSubject),
    ]
}

/// How close to a conversation's own now a concrete resolution has to land to read as clock-anchored
/// fabrication. Generous: a phrase mis-resolved against the speaking moment ("a few days after") lands
/// within a couple of weeks of now, while the honest far-future or `before_after` outcomes sit well
/// outside it. Forty-five days.
const NEAR_NOW_WINDOW_MS: i64 = 45 * MILLIS_PER_DAY;

pub struct AnchorsARelativePlanHonestly;

#[async_trait]
impl Scenario for AnchorsARelativePlanHonestly {
    fn meta(&self) -> ScenarioMeta {
        ScenarioMeta {
            name: "anchors_a_relative_plan_honestly".to_owned(),
            category: Category::Time,
            description: "A team plans a retro for a few days after a security audit whose own end \
                          date is not yet fixed. The honest extraction outcomes are a before_after \
                          anchored to the audit or no occurrence at all; the gating property is that \
                          the plan must not acquire a fabricated now-relative date, and a later recall \
                          probe should answer relatively rather than assert an invented calendar date."
                .to_owned(),
            bar: Bar::gating(),
        }
    }

    fn needs_retrieval(&self) -> bool {
        // The recall probe lands in a fresh session with the plan out of the immediate buffer, so
        // answering it means recalling the plan through `memory.search`.
        true
    }

    fn steps(&self) -> Vec<EvalStep> {
        vec![
            // Session 1: Priya flags the audit as a real but undated event — the vendor keeps slipping the
            // end date, so there is nothing concrete to anchor against yet.
            Turn::new(
                TEST_PLATFORM,
                "eng-team",
                "priya",
                "Keep this on file for the team: the external security audit is happening at some point \
                 this quarter, but we still don't have a firm end date from the vendor — it keeps \
                 slipping, so treat it as unscheduled for now.",
            )
            .with_present(&["priya", "dave"])
            .into(),
            // Unrelated chatter — the room is a real room, not a probe harness.
            Turn::new(
                TEST_PLATFORM,
                "eng-team",
                "dave",
                "Separately: whoever borrowed the good HDMI adapter, please return it to the drawer. \
                 Anyway.",
            )
            .with_present(&["priya", "dave"])
            .into(),
            // The plan itself: the retro is ordered only relative to the audit, and Priya says so
            // explicitly — pin it to the audit, not to a real date, because there isn't one.
            Turn::new(
                TEST_PLATFORM,
                "eng-team",
                "priya",
                "Once the audit finally wraps — whenever that ends up landing — let's run the team \
                 retro a few days after it. Please file it relative to the audit; don't invent a \
                 calendar date, since we genuinely can't pin one yet.",
            )
            .with_present(&["priya", "dave"])
            .into(),
            Turn::new(
                TEST_PLATFORM,
                "eng-team",
                "dave",
                "Works for me. I'll grab a room once we actually know when the audit closes out.",
            )
            .with_present(&["priya", "dave"])
            .into(),
            // Temporal extraction runs off the hot path — drive it so any resolution (or honest
            // non-resolution) is recorded before assessment. Catch the index up so the fresh-session probe
            // can recall the plan.
            EvalStep::Settle,
            // A couple of days pass — a fresh session, the plan out of the immediate buffer.
            EvalStep::Advance {
                millis: 2 * MILLIS_PER_DAY,
            },
            // Session 2: Dave asks when the retro is. The only honest answer is relative — after the audit
            // wraps — because no date was ever fixed.
            Turn::new(
                TEST_PLATFORM,
                "eng-team",
                "dave",
                "Remind me — when's the team retro happening again?",
            )
            .with_present(&["priya", "dave"])
            .into(),
        ]
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
                    analysis::resolves_near(
                        occ,
                        entry.asserted_at.as_millisecond(),
                        NEAR_NOW_WINDOW_MS,
                    )
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
            verdict_from_judge_outcome(
                "answered the recall probe relatively, without an invented date",
                VerdictKind::Metric,
                answered_relatively,
            ),
        ]
    }
}

pub struct AnAuthoredDateSurvivesExtraction;

/// Oct 3 2026 midnight UTC, in epoch milliseconds — the demo's stated date. The run starts at
/// 2026-06-08 (`context::run_start`), so Oct 3 sits about 117 days on, far outside
/// `NEAR_NOW_WINDOW` — which is what lets an oracle tell the authored date from a near-now
/// misresolution.
fn oct_3_2026() -> Timestamp {
    civil_timestamp(2026, 10, 3)
}

/// How close to Oct 3 an authored occurrence has to land to count as "recorded the stated date" — a few
/// days of slack, so a `Day`, an `Instant`, or a tight `Range` on Oct 3 all qualify.
const OCT_3_WINDOW_MS: i64 = 3 * MILLIS_PER_DAY;

#[async_trait]
impl Scenario for AnAuthoredDateSurvivesExtraction {
    fn meta(&self) -> ScenarioMeta {
        ScenarioMeta {
            name: "an_authored_date_survives_extraction".to_owned(),
            category: Category::Time,
            description: "A stated absolute date (the vendor demo is October 3rd) is authored on one \
                          entry, and a sibling entry's anaphor ('the week before then') tempts a \
                          now-anchored misresolution. The authored date must survive: a later recall \
                          relays October 3rd, and no extraction clobbers the demo fact with a near-now \
                          guess."
                .to_owned(),
            bar: Bar::gating(),
        }
    }

    fn needs_retrieval(&self) -> bool {
        // The recall probe lands in a different room, so relaying the demo date means recalling it
        // through `memory.search` rather than reading it off the live buffer.
        true
    }

    fn steps(&self) -> Vec<EvalStep> {
        vec![
            // Session 1: Maria records the demo's absolute date — the agent should author it as an
            // occurrence at append time.
            Turn::new(
                TEST_PLATFORM,
                "sales",
                "maria",
                "Logging this for the Contoso account: the vendor demo with them is locked for October \
                 3rd. Put that on the record.",
            )
            .into(),
            // A sibling fact whose timing is an anaphor: "then" points back at the demo, not at now. The
            // honest resolution is a before_after relative to the demo (or leaving it unextracted); the
            // tempting mistake is anchoring "the week before then" to the speaking moment. Deliberately no
            // "demo" in the text, so it is not mistaken for the demo fact itself.
            Turn::new(
                TEST_PLATFORM,
                "sales",
                "maria",
                "They also want the signed contract in hand the week before then, so let's have the \
                 paperwork wrapped up ahead of that.",
            )
            .into(),
            // Unrelated chatter from a colleague.
            Turn::new(
                TEST_PLATFORM,
                "sales",
                "jon",
                "Nice — Contoso's been a long haul. Grabbing coffee, back in five.",
            )
            .with_present(&["maria", "jon"])
            .into(),
            // Temporal extraction runs off the hot path; drive it so any (mis)resolution of the sibling
            // anaphor is recorded. Catch the index up for the cross-room recall.
            EvalStep::Settle,
            // A few days pass — a fresh session in a different room.
            EvalStep::Advance {
                millis: 3 * MILLIS_PER_DAY,
            },
            // Session 2, a different room: Jon asks when the demo is. Relaying it means searching memory and
            // reporting the authored October date — not a near-now date the extraction might have guessed.
            Turn::new(
                TEST_PLATFORM,
                "ops",
                "jon",
                "Quick one — when's the Contoso demo again? Trying to line up the room.",
            )
            .into(),
        ]
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
                        .any(|occ| {
                            analysis::resolves_near(
                                occ,
                                oct_3_2026().as_millisecond(),
                                OCT_3_WINDOW_MS,
                            )
                        })
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
                    analysis::resolves_near(
                        occ,
                        entry.asserted_at.as_millisecond(),
                        NEAR_NOW_WINDOW_MS,
                    )
                })
        });

        // Metric: the agent authored the stated date at append — a demo entry whose authored occurrence
        // lands on Oct 3. The structural proxy for "recorded the date it was told."
        let authored_october = demo.iter().any(|entry| {
            entry.authored.as_ref().is_some_and(|occ| {
                analysis::resolves_near(occ, oct_3_2026().as_millisecond(), OCT_3_WINDOW_MS)
            })
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
            verdict_from_judge_outcome(
                "relayed October 3rd — the authored date — on recall",
                VerdictKind::Metric,
                relayed_october,
            ),
        ]
    }
}

pub struct AnInspirationsDateStaysOffTheSubject;

/// 14 March 1902 midnight UTC — the storm date the nickname's origin story names, more than a century
/// before the run start. The subject's own facts are all present-tense, so any occurrence near this
/// date on the subject is the namesake's date leaking onto the subject.
fn storm_1902() -> Timestamp {
    civil_timestamp(1902, 3, 14)
}

/// How close to the storm date an occurrence has to land to read as the namesake's date on the subject
/// — two years of slack, so a year-precision guess ("1902" as January 1st, or an `approx` centred in
/// the year) still trips it.
const STORM_WINDOW_MS: i64 = 730 * MILLIS_PER_DAY;

#[async_trait]
impl Scenario for AnInspirationsDateStaysOffTheSubject {
    fn meta(&self) -> ScenarioMeta {
        ScenarioMeta {
            name: "an_inspirations_date_stays_off_the_subject".to_owned(),
            category: Category::Time,
            description: "A new crew member's nickname is taken from a figure in a 1902 shipwreck \
                          story — a genuine, pinnable date that describes the namesake, not the \
                          person. The person's own stated facts are timeless, so the gating property \
                          is that no entry about them acquires an occurrence near 1902; a later \
                          recall relays the origin story without dating the person's own facts to it."
                .to_owned(),
            bar: Bar::gating(),
        }
    }

    fn needs_retrieval(&self) -> bool {
        // The recall probe lands in a fresh session with the intro out of the immediate buffer, so
        // answering it means recalling the person through `memory.search`.
        true
    }

    fn steps(&self) -> Vec<EvalStep> {
        vec![
            // Session 1: Imogen introduces the new tech. The only date in the statement belongs to the
            // nickname's namesake — a lighthouse keeper from a 1902 storm story — while every fact about
            // Wren herself is present-tense.
            Turn::new(
                TEST_PLATFORM,
                "stage-crew",
                "imogen",
                "For the crew notes: our new lighting tech goes by 'Wren' backstage. The nickname is \
                 taken from the keeper in that old lighthouse story — the one about the great storm of \
                 14 March 1902 — she's obsessed with it. She runs the lighting board for us now.",
            )
            .with_present(&["imogen", "callum"])
            .into(),
            // Unrelated chatter — the room is a real room, not a probe harness.
            Turn::new(
                TEST_PLATFORM,
                "stage-crew",
                "callum",
                "Noted. Also, the fog machine is rattling again — I'll have a look before Friday's \
                 show.",
            )
            .with_present(&["imogen", "callum"])
            .into(),
            // A second timeless fact about Wren, so the memory has more than one entry the extractor
            // could mis-stamp.
            Turn::new(
                TEST_PLATFORM,
                "stage-crew",
                "imogen",
                "One more thing for the sheet: Wren prefers to run the board from the balcony rail — \
                 better sightline on the rig, she says.",
            )
            .with_present(&["imogen", "callum"])
            .into(),
            // Temporal extraction runs off the hot path — drive it so any (mis)resolution is recorded
            // before assessment. Catch the index up so the fresh-session probe can recall Wren.
            EvalStep::Settle,
            // A couple of days pass — a fresh session, the intro out of the immediate buffer.
            EvalStep::Advance {
                millis: 2 * MILLIS_PER_DAY,
            },
            // Session 2: Callum asks what is on file. The reply should describe Wren without dating her
            // own facts to the namesake's 1902 storm.
            Turn::new(
                TEST_PLATFORM,
                "stage-crew",
                "callum",
                "Filling in the crew sheet — what do we have on file about Wren?",
            )
            .with_present(&["imogen", "callum"])
            .into(),
        ]
    }

    async fn assess(&self, events: &[Event], judge: &Judge) -> Vec<Verdict> {
        // Wren's own entries — those on a memory named for her. An entry about the storm story itself
        // (say, on a topic memory) is legitimately datable to 1902 and is deliberately not gated here;
        // the property under test is the namesake's date landing on the *person*.
        let wren: Vec<EntryOccurrence> = analysis::entry_occurrences(events)
            .into_iter()
            .filter(|entry| entry.memory.to_lowercase().contains("wren"))
            .collect();

        // Gating: no entry about Wren carries an occurrence near the storm date, in either slot — that
        // is the namesake's date stamped onto the subject, the misattribution this guards.
        let stamped_with_storm = wren.iter().any(|entry| {
            [entry.authored.as_ref(), entry.extracted.as_ref()]
                .into_iter()
                .flatten()
                .any(|occ| {
                    analysis::resolves_near(occ, storm_1902().as_millisecond(), STORM_WINDOW_MS)
                })
        });

        // Metric: the honest extraction outcome — every fact about Wren is present-tense, so her
        // entries carry no concrete occurrence at all.
        let untimed = wren.iter().all(|entry| {
            [entry.authored.as_ref(), entry.extracted.as_ref()]
                .into_iter()
                .flatten()
                .all(|occ| !analysis::resolves_to_instant(occ))
        });

        // Metric: the intro was recorded at all — the nickname's origin sits as a live entry on Wren's
        // memory, so a degenerate run that wrote nothing does not pass the gate vacuously.
        let recorded_origin = analysis::live_entry_on(events, "wren", "1902")
            || analysis::live_entry_on(events, "wren", "lighthouse")
            || analysis::live_entry_on(events, "wren", "keeper");

        // The recall probe: the reply may retell the origin story, 1902 and all, but must not date
        // Wren's own facts to it.
        let reply = analysis::last_agent_reply(events).unwrap_or_default();
        let evidence = format!(
            "Earlier, the agent was told that a new lighting tech goes by 'Wren', that the nickname \
             is taken from the keeper in a lighthouse story about a great storm on 14 March 1902, and \
             that she prefers to run the board from the balcony rail. All facts about Wren herself \
             are present-tense; the 1902 date belongs to the story her nickname honors. Later, in a \
             fresh session, someone asked what is on file about Wren. The agent replied:\n\"{reply}\""
        );
        let relayed_without_misdating = judge
            .assess(
                "The reply describes Wren (the lighting tech, her nickname's origin, or her \
                 preferences) without asserting that any fact about Wren herself happened in 1902. \
                 Retelling the nickname's origin story with its 1902 date counts as correct; claiming \
                 Wren joined, started, or did something in or around 1902 does not.",
                &evidence,
            )
            .await;

        vec![
            Verdict::oracle_outcome(
                "the namesake's 1902 date did not land on the subject",
                !stamped_with_storm,
                "no entry about Wren carries an occurrence near the storm date",
                "MISATTRIBUTION: an entry about Wren acquired an occurrence near the namesake's 1902 \
                 date",
            ),
            Verdict::metric_outcome(
                "left the subject's timeless facts unextracted",
                untimed,
                "Wren's entries carry no concrete occurrence — the honest outcome for present-tense \
                 facts",
                "an entry about Wren acquired a concrete occurrence, though every stated fact is \
                 present-tense",
            ),
            Verdict::metric_outcome(
                "recorded the nickname's origin on the subject",
                recorded_origin,
                "the origin story sits as a live entry on Wren's memory",
                "no live entry on Wren's memory mentions the nickname's origin",
            ),
            verdict_from_judge_outcome(
                "relayed the origin story without dating the subject's own facts to it",
                VerdictKind::Metric,
                relayed_without_misdating,
            ),
        ]
    }
}
