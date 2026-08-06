//! The withdrawal half of the anchor rule: an occurrence already sitting on an entry, dated to a
//! referent the statement merely mentions, is retracted by the turn-end pass rather than left standing.
//!
//! [`AnInspirationsDateStaysOffTheSubject`](super::temporal::AnInspirationsDateStaysOffTheSubject)
//! covers the same misattribution on the way in — the agent should not write that date in the first
//! place — and it usually does not: across the last twenty-five recorded runs of that scenario the
//! agent authored the namesake's date onto the subject exactly once. That rarity is good behaviour and
//! a poor test. Waiting for the slip to reproduce needs roughly sixty runs to see one instance, and a
//! run that never triggers it reports a perfect score while exercising nothing.
//!
//! So this scenario seeds the mistake instead of hoping for it. The entry arrives already dated, as
//! though an earlier turn recorded it wrongly, and the property under test is purely the pass's: shown
//! a statement whose own subject is timeless beside a date belonging to its namesake, does it report
//! the entry as misdated. Seeding also puts the entry at the visibility the failure actually occurs at
//! — `Attributed`, where a fact relayed *about* someone lands — which is the shape that reaches only
//! the focused non-public pass.
//!
//! The bar is a metric, not a gate. Withdrawal is a should-surface judgement the model makes about
//! language, with the error band that implies; the must-not-surface half (never fabricating a date) is
//! gated where it belongs, on the scenarios that guard writing one. The threshold sits at 0.7 against a
//! first measured rate of 19/20 — low enough not to flap on the model's own variance, high enough that
//! the instruction quietly ceasing to land would show.

use std::sync::Arc;

use async_trait::async_trait;
use zuihitsu::{
    CivilDate, Event, EventPayload, MemoryId, MemoryName, TEST_PLATFORM, Teller, TemporalRef,
    Timestamp, Visibility, ids::EntryId,
};

use crate::{
    analysis::{self, EntryOccurrence},
    context::civil_timestamp,
    judge::Judge,
    package::{Bar, Category, ScenarioMeta, Verdict},
    scenario::Scenario,
    step::{EvalStep, Turn},
};

pub struct AMisdatedOccurrenceIsWithdrawn;

/// The subject: a person whose nickname comes from a story, not a person who did anything in 1902.
const SUBJECT: &str = "person/wren";

/// The seeded entry — a fact about the subject that *mentions* a date belonging to the namesake. The
/// sentence is the shape the misattribution takes: the subject's own claim is present-tense ("goes by",
/// "is named for"), and the only pinnable date in it is the story's.
const MISDATED_TEXT: &str =
    "is named for the lighthouse keeper in the story of the great storm of 14 March 1902";

/// How near the storm date an occurrence has to sit to count as still carrying it.
const STORM_WINDOW_MS: i64 = 30 * 24 * 60 * 60 * 1000;

fn storm_1902() -> Timestamp {
    civil_timestamp(1902, 3, 14)
}

#[async_trait]
impl Scenario for AMisdatedOccurrenceIsWithdrawn {
    fn meta(&self) -> ScenarioMeta {
        ScenarioMeta {
            name: "a_misdated_occurrence_is_withdrawn".to_owned(),
            category: Category::Time,
            description: "An entry about a person arrives already dated to the namesake its text \
                          mentions. The turn-end pass should report it as misdated and withdraw the \
                          occurrence, leaving the entry untimed rather than claiming the person's own \
                          fact happened in 1902."
                .to_owned(),
            bar: Bar::Metric { threshold: 0.7 },
        }
    }

    fn steps(&self) -> Vec<EvalStep> {
        let subject = MemoryId::generate();
        let teller = MemoryId::generate();
        let now = civil_timestamp(2026, 6, 8);
        let seed = vec![
            EventPayload::memory_created(subject, MemoryName::new(SUBJECT)),
            EventPayload::memory_created(teller, MemoryName::new("person/imogen")),
            EventPayload::participant_identified(teller, TEST_PLATFORM, "imogen"),
            // The mistake, pre-made: the namesake's date stamped onto the subject's own entry, at the
            // visibility a relayed fact about someone actually lands at.
            EventPayload::MemoryContentAppended {
                id: subject,
                entry_id: EntryId::generate(),
                asserted_at: now,
                occurred_at: Some(TemporalRef::Day(CivilDate("1902-03-14".into()))),
                text: MISDATED_TEXT.to_owned(),
                told_by: Teller::Participant(teller),
                told_in: None,
                visibility: Visibility::Attributed,
            },
        ];
        vec![
            EvalStep::SeedEvents(seed),
            // A turn that touches the subject, so the turn-end pass runs over its memory and meets the
            // seeded entry. The remark is deliberately timeless: nothing here should acquire a date, so
            // the only occurrence in play at the end is the seeded one.
            Turn::new(
                TEST_PLATFORM,
                "crew",
                "imogen",
                "Wren's taken over the lighting board for us — she runs it from the balcony rail now.",
            )
            .into(),
            // The turn-end pass is lazy: it catches up on memories touched *before* the current turn, so
            // a scenario that ends on the turn it cares about never has that turn's memory described.
            // This drives the catch-up explicitly, which is what puts the seeded entry in front of the
            // pass at all.
            EvalStep::DescribeCatchUp,
        ]
    }

    async fn assess(&self, events: &[Event], _judge: &Judge) -> Vec<Verdict> {
        // Fixture sanity: the seeded entry must exist and have arrived dated, or the run tests nothing.
        let seeded_arrived_dated = events.iter().any(|event| {
            matches!(
                &event.payload,
                EventPayload::MemoryContentAppended { text, occurred_at: Some(_), .. }
                    if text == MISDATED_TEXT
            )
        });
        assert!(
            seeded_arrived_dated,
            "the seed must place the misdated entry, dated, or the withdrawal has nothing to retract",
        );

        let subject_entries: Vec<EntryOccurrence> = analysis::entry_occurrences(events)
            .into_iter()
            .filter(|entry| entry.memory == SUBJECT)
            .collect();

        // The property: no entry on the subject still carries the storm date. The pass reports the
        // seeded entry as misdated, the occurrence is withdrawn, and the entry returns to untimed.
        let still_dated = subject_entries.iter().any(|entry| {
            [entry.authored.as_ref(), entry.extracted.as_ref()]
                .into_iter()
                .flatten()
                .any(|occ| {
                    analysis::resolves_near(occ, storm_1902().as_millisecond(), STORM_WINDOW_MS)
                })
        });

        // The counter-property, reported alongside: withdrawal is not licence to strip the memory. The
        // seeded entry itself must survive as live text — retracting a date is not retracting a fact.
        let text_survives = analysis::live_entry_on(events, "wren", "lighthouse")
            || analysis::live_entry_on(events, "wren", "keeper");

        vec![
            Verdict::metric_outcome(
                "withdrew the namesake's date from the subject's entry",
                !still_dated,
                "the seeded occurrence was retracted, leaving the entry untimed",
                "the entry still carries the namesake's 1902 date",
            ),
            Verdict::metric_outcome(
                "kept the entry's text while dropping its date",
                text_survives,
                "the fact survives as live text with its occurrence withdrawn",
                "the entry's text did not survive — a withdrawal must retract the date, not the fact",
            ),
        ]
    }
}

pub fn scenarios() -> Vec<Arc<dyn Scenario>> {
    vec![Arc::new(AMisdatedOccurrenceIsWithdrawn)]
}
