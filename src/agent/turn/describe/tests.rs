use std::collections::BTreeMap;

use crate::{
    agent::turn::describe::{
        extract::ExtractedTime,
        occurrences::{quotes_the_statement, states_a_time},
        synthesis::statements_prompt,
    },
    event::{Teller, Visibility, Volatility},
    graph::{AttestationView, EntryOrigin, EntryView, MemoryView},
    ids::{EntryId, MemoryId, MemoryName},
    time::{self, CivilDate, Rrule, TemporalRef, Timestamp},
};

fn ms(date: &str) -> i64 {
    time::civil_date_to_millis(date).unwrap()
}

/// A `Public` attestation by a participant — a corroborating source for the `attested by` clause.
fn attestation(teller: Teller, posture: Visibility) -> AttestationView {
    AttestationView::founding(
        teller,
        None,
        Timestamp::from_millis(ms("2026-06-08")),
        posture,
    )
}

/// A minimal agent-told entry carrying `text` and an optional occurrence, for the prompt-shape tests.
fn entry(text: &str, occurred_at: Option<TemporalRef>) -> EntryView {
    EntryView {
        entry_id: EntryId::generate(),
        asserted_at: Timestamp::from_millis(ms("2026-06-08")),
        occurred_sort: None,
        occurred_at,
        occurred_authored: false,
        text: text.to_owned(),
        told_by: Teller::Agent,
        told_in: None,
        visibility: Visibility::Public,
        superseded_by: None,
        retracted_reason: None,
        origin: EntryOrigin::Recorded,
        attestations: Vec::new(),
    }
}

fn memory(name: &str) -> MemoryView {
    MemoryView {
        id: MemoryId::generate(),
        name: MemoryName::new(name),
        description: String::new(),
        volatility: Volatility::Medium,
        created_at: Timestamp::from_millis(ms("2026-06-08")),
        tags: Vec::new(),
    }
}

#[test]
fn statements_prompt_annotates_a_dated_statement_and_leaves_an_undated_one() {
    let memory = memory("event/demo");
    let entries = [
        entry(
            "Vendor demo",
            Some(TemporalRef::Day(CivilDate("2026-10-03".into()))),
        ),
        entry("The demo is locked for this date.", None),
    ];
    let prompt = statements_prompt(
        &memory,
        &entries,
        &BTreeMap::new(),
        Timestamp::from_millis(ms("2026-06-08")),
    );
    // The dated statement's bracket carries its occurrence, so a back-pointing phrase in a sibling resolves against
    // it rather than the conversation's now.
    assert!(prompt.contains("1. [from the agent · Mon 08 Jun · occurred 2026-10-03] Vendor demo"));
    // The undated statement's bracket is unchanged — no occurrence, no trailing annotation.
    assert!(prompt.contains("2. [from the agent · Mon 08 Jun] The demo is locked for this date."));
}

#[test]
fn statements_prompt_notes_a_multiply_attested_statement_and_ignores_a_hidden_one() {
    let memory = memory("topic/hooli");
    let (erin, dave, frank) = (
        MemoryId::generate(),
        MemoryId::generate(),
        MemoryId::generate(),
    );
    let teller_names: BTreeMap<MemoryId, String> = [
        (erin, "person/erin".to_owned()),
        (dave, "person/dave".to_owned()),
        (frank, "person/frank".to_owned()),
    ]
    .into_iter()
    .collect();

    // The launch slip: erin's public account, dave's attributed corroboration, and frank's hidden
    // private confidence. The clause counts erin and dave; frank's private endorsement never bumps it.
    let mut multi = entry("The launch slipped", None);
    multi.told_by = Teller::Participant(erin);
    multi.attestations = vec![
        attestation(Teller::Participant(erin), Visibility::Public),
        attestation(Teller::Participant(dave), Visibility::Attributed),
        attestation(Teller::Participant(frank), Visibility::PrivateToTeller),
    ];
    // A single-source sibling: erin alone, so no clause.
    let mut single = entry("The venue is booked", None);
    single.told_by = Teller::Participant(erin);
    single.attestations = vec![attestation(Teller::Participant(erin), Visibility::Public)];

    let prompt = statements_prompt(
        &memory,
        &[multi, single],
        &teller_names,
        Timestamp::from_millis(ms("2026-06-08")),
    );
    assert!(
        prompt.contains("attested by person/erin, person/dave"),
        "{prompt}"
    );
    // The hidden private endorsement leaves no residue in the durable prompt prose.
    assert!(!prompt.contains("person/frank"), "{prompt}");
    // The single-source statement carries no clause.
    assert!(
        prompt.contains("2. [from person/erin · Mon 08 Jun] The venue is booked"),
        "{prompt}"
    );
}

#[test]
fn a_cue_quoted_from_the_statement_passes_the_span_check() {
    let text = "Met Dave at the gym last Tuesday, and the migration ships next Friday.";
    assert!(quotes_the_statement("last Tuesday", text));
    assert!(quotes_the_statement("next Friday", text));
    // Case, surrounding punctuation, and runs of whitespace are all tidying a faithful quote picks up,
    // so the fold ignores them rather than failing an honest cue.
    assert!(quotes_the_statement("LAST TUESDAY", text));
    assert!(quotes_the_statement("\"last Tuesday\"", text));
    assert!(quotes_the_statement("last  Tuesday", text));
    assert!(quotes_the_statement("ships next Friday.", text));
    // A cue spanning the comma still folds to the same word sequence.
    assert!(quotes_the_statement("gym last Tuesday, and", text));
}

#[test]
fn a_cue_the_statement_does_not_contain_fails_the_span_check() {
    let text = "Met Dave at the gym last Tuesday.";
    // The date the model resolved to is not a quote of anything.
    assert!(!quotes_the_statement("2026-06-02", text));
    // A plausible time phrase the statement never used — the fabrication the check exists to catch.
    assert!(!quotes_the_statement("next Friday", text));
    // A paraphrase is not a quote, however faithful its meaning.
    assert!(!quotes_the_statement("the previous Tuesday", text));
    // Words present but not contiguous do not make a span.
    assert!(!quotes_the_statement("Met Tuesday", text));
    // An empty cue stands behind nothing.
    assert!(!quotes_the_statement("", text));
    assert!(!quotes_the_statement("   ", text));
}

/// 2026-06-08 is a Monday, which the weekday cases below turn on.
fn monday() -> Timestamp {
    Timestamp::from_millis(ms("2026-06-08"))
}

#[test]
fn a_statement_naming_a_time_supports_a_current_day_resolution() {
    let now = monday();
    // A written date, in the formats a statement actually carries one in.
    assert!(states_a_time("Signed the lease on 2026-06-08", now));
    assert!(states_a_time("Closes 8 June", now));
    assert!(states_a_time("Closes June 8", now));
    assert!(states_a_time("Rent is due on the 8th", now));
    assert!(states_a_time("Standup moved to 9:30", now));
    assert!(states_a_time("Doors at 7pm", now));
    // A same-day deictic puts the speaker's own day in the statement.
    assert!(states_a_time("The surveyor called this morning", now));
    assert!(states_a_time("It kicked off today.", now));
    assert!(states_a_time("The ferry leaves tonight", now));
    // Case is irrelevant — the cue is the word, not its rendering.
    assert!(states_a_time("TODAY the vendor confirmed", now));
}

#[test]
fn a_statement_naming_no_time_does_not_support_a_current_day_resolution() {
    let now = monday();
    // An intention waiting on a date nobody holds: said today, but about no day at all.
    assert!(!states_a_time(
        "Keen to help with setup once the room is sorted.",
        now
    ));
    // A standing fact is timeless — true before today and after it.
    assert!(!states_a_time("Leads the volcano project", now));
    // The vague now-words anchor loosely to the moment of speaking without pinning a day, so they are
    // grounds for omitting rather than for dating; treating them as support would readmit exactly the
    // fabrication the guard exists to catch.
    assert!(!states_a_time(
        "Joined recently as the new lighting tech",
        now
    ));
    assert!(!states_a_time("Has been travelling lately", now));
    assert!(!states_a_time("Will send the invoice soon", now));
    assert!(!states_a_time("Currently between jobs", now));
    // A bare month names a month, not a day, so it cannot authorize the current day on its own.
    assert!(!states_a_time("The launch is sometime in June", now));
}

#[test]
fn a_bare_number_that_is_not_a_date_does_not_support_a_current_day_resolution() {
    let now = monday();
    // Floors, quarters, versions, counts, and street numbers are far commoner than dates, and each of
    // these was observed keeping a fabricated current-day occurrence when any digit counted as a time.
    assert!(!states_a_time(
        "The coffee machine on floor 3 is out of beans.",
        now
    ));
    assert!(!states_a_time(
        "Dave is thinking through the Q3 roadmap.",
        now
    ));
    assert!(!states_a_time("Runs the v2 migration", now));
    assert!(!states_a_time("Has 3 kids", now));
    // A bare year names no day either, and appears in event names constantly.
    assert!(!states_a_time(
        "The 2026 offsite is happening in Denver",
        now
    ));
    assert!(!states_a_time("Lives at 42 Fitzroy St", now));
}

#[test]
fn a_weekday_supports_a_current_day_resolution_only_on_that_weekday() {
    // Said on the Monday it names, "Monday" is why a resolution landed on the current day.
    assert!(states_a_time("Moved the standup to Monday", monday()));
    // Said on a Monday, "Thursday" cannot be — so it does not disarm the guard. This is the case the
    // extraction prompt teaches with a standing weekly fact, which must stay untimed.
    assert!(!states_a_time("Moved the standup to Thursday", monday()));
    assert!(!states_a_time("Runs the Tuesday reading group", monday()));
}

#[test]
fn instant_date_only_coerces_to_day() {
    // The model uses `instant` for bare days; a date-only value becomes a `Day`, not an `Instant`.
    assert_eq!(
        ExtractedTime::Instant("2026-06-03".to_owned()).into_temporal_ref(),
        Some(TemporalRef::Day(CivilDate("2026-06-03".into())))
    );
}

#[test]
fn instant_with_a_time_stays_an_instant() {
    let at = time::date_or_datetime_to_millis("2026-06-02T09:30:00Z").unwrap();
    assert_eq!(
        ExtractedTime::Instant("2026-06-02T09:30:00Z".to_owned()).into_temporal_ref(),
        Some(TemporalRef::Instant(Timestamp::from_millis(at)))
    );
}

#[test]
fn day_maps_through() {
    assert_eq!(
        ExtractedTime::Day("2026-06-03".to_owned()).into_temporal_ref(),
        Some(TemporalRef::Day(CivilDate("2026-06-03".into())))
    );
}

#[test]
fn range_and_approx_convert_dates_to_millis() {
    assert_eq!(
        ExtractedTime::Range {
            start: "2019-01-01".to_owned(),
            end: "2019-12-31".to_owned(),
        }
        .into_temporal_ref(),
        Some(TemporalRef::Range {
            start: Timestamp::from_millis(ms("2019-01-01")),
            end: Timestamp::from_millis(ms("2019-12-31")),
        })
    );
    assert_eq!(
        ExtractedTime::Approx {
            center: "2024-06-07".to_owned(),
            fuzz_days: 60,
        }
        .into_temporal_ref(),
        Some(TemporalRef::Approx {
            center: Timestamp::from_millis(ms("2024-06-07")),
            fuzz_days: 60,
        })
    );
}

#[test]
fn before_after_parses_direction_case_insensitively() {
    assert_eq!(
        ExtractedTime::BeforeAfter {
            dir: "After".to_owned(),
            anchor: "event/wedding".to_owned(),
        }
        .into_temporal_ref(),
        Some(TemporalRef::after(MemoryName::new("event/wedding")))
    );
    // An unrecognized direction drops the occurrence rather than guessing.
    assert_eq!(
        ExtractedTime::BeforeAfter {
            dir: "sideways".to_owned(),
            anchor: "x".to_owned(),
        }
        .into_temporal_ref(),
        None
    );
}

#[test]
fn malformed_dates_drop() {
    // 2026 is not a leap year, so Feb 29 is impossible; a non-date instant has no datetime either.
    assert_eq!(
        ExtractedTime::Day("2026-02-29".to_owned()).into_temporal_ref(),
        None
    );
    assert_eq!(
        ExtractedTime::Instant("whenever".to_owned()).into_temporal_ref(),
        None
    );
    assert_eq!(
        ExtractedTime::Range {
            start: "nope".to_owned(),
            end: "2020-01-01".to_owned(),
        }
        .into_temporal_ref(),
        None
    );
}

#[test]
fn a_supported_recurrence_is_kept_and_a_free_phrase_is_dropped() {
    // A well-formed rule arms a wake-up, so it is committed.
    assert_eq!(
        ExtractedTime::Recurring("FREQ=WEEKLY;BYDAY=MO".to_owned()).into_temporal_ref(),
        Some(TemporalRef::Recurring(Rrule("FREQ=WEEKLY;BYDAY=MO".into())))
    );
    // A free-phrased cadence ("every Monday") is not an rrule this build interprets: dropping it
    // here leaves the entry untimed, rather than committing a Recurring that silently never fires.
    assert_eq!(
        ExtractedTime::Recurring("every Monday".to_owned()).into_temporal_ref(),
        None
    );
    assert_eq!(
        ExtractedTime::Recurring("FREQ=HOURLY".to_owned()).into_temporal_ref(),
        None
    );
}
