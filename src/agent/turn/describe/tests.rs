use super::extract::ExtractedTime;
use crate::{
    ids::MemoryName,
    time::{self, CivilDate, Rrule, TemporalRef, Timestamp},
};

fn ms(date: &str) -> i64 {
    time::civil_date_to_millis(date).unwrap()
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
    let at = time::datetime_to_millis("2026-06-02T09:30:00Z").unwrap();
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
