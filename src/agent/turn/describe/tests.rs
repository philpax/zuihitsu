use std::collections::BTreeMap;

use super::{extract::ExtractedTime, synthesis::statements_prompt};
use crate::{
    event::{Teller, Visibility, Volatility},
    graph::{EntryOrigin, EntryView, MemoryView},
    ids::{EntryId, MemoryId, MemoryName},
    time::{self, CivilDate, Rrule, TemporalRef, Timestamp},
};

fn ms(date: &str) -> i64 {
    time::civil_date_to_millis(date).unwrap()
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
