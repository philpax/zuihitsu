use super::{
    MILLIS_PER_DAY, MILLIS_PER_WEEK, Rrule, Timestamp, add_days, add_months, civil_date_to_millis,
    day_window, next_occurrence, next_weekday, parse_duration_millis, rrule_is_supported, today,
    weekday,
};

/// `next_occurrence` against a `dtstart`/`after` given in epoch millis, for brevity.
fn next(rule: &str, dtstart: i64, after: i64) -> Option<i64> {
    next_occurrence(
        &Rrule(rule.into()),
        Timestamp::from_millis(dtstart),
        Timestamp::from_millis(after),
    )
    .map(Timestamp::as_millis)
}

/// Midnight-UTC millis of a `YYYY-MM-DD` day (the tests' calendar anchors).
fn day(date: &str) -> i64 {
    civil_date_to_millis(date).unwrap()
}

/// The agent-facing date arithmetic: day/month shifts (with month clamping), weekday lookup, and
/// the next-weekday resolution that replaces "compute this Friday's date" in the model's head.
#[test]
fn date_helpers_compute_relative_days() {
    // 2026-06-08 is a Monday, the suite's anchor.
    let monday = Timestamp::from_millis(day("2026-06-08"));
    assert_eq!(today(monday), "2026-06-08");
    assert_eq!(weekday("2026-06-08").as_deref(), Some("Monday"));

    // "this Friday" from Monday is +4 days, never +5 (the off-by-one the model slipped on).
    assert_eq!(
        next_weekday(monday, "friday").as_deref(),
        Some("2026-06-12")
    );
    assert_eq!(
        next_weekday(monday, "Friday").as_deref(),
        Some("2026-06-12")
    );
    // The current weekday resolves to today, not a week out.
    assert_eq!(
        next_weekday(monday, "monday").as_deref(),
        Some("2026-06-08")
    );
    assert_eq!(next_weekday(monday, "notaday"), None);

    assert_eq!(add_days("2026-06-08", 4).as_deref(), Some("2026-06-12"));
    assert_eq!(add_days("2026-06-08", -8).as_deref(), Some("2026-05-31"));
    // Month arithmetic clamps the day where the target month is shorter.
    assert_eq!(add_months("2026-01-31", 1).as_deref(), Some("2026-02-28"));
    assert_eq!(add_months("2026-06-08", 2).as_deref(), Some("2026-08-08"));
    assert_eq!(add_days("not-a-date", 1), None);
}

#[test]
fn weekly_recurrence_finds_the_next_instance_after() {
    // Anchored at 0: occurrences at 0, 1 week, 2 weeks, …
    assert_eq!(next("FREQ=WEEKLY", 0, 0), Some(MILLIS_PER_WEEK));
    assert_eq!(
        next("FREQ=WEEKLY", 0, 10 * MILLIS_PER_DAY),
        Some(2 * MILLIS_PER_WEEK)
    );
    // Strictly after: exactly on an instant advances to the next.
    assert_eq!(
        next("FREQ=WEEKLY", 0, MILLIS_PER_WEEK),
        Some(2 * MILLIS_PER_WEEK)
    );
}

#[test]
fn daily_recurrence_honors_interval() {
    // Every two days from 0: 0, 2, 4, … so just after day 3 the next is day 4.
    assert_eq!(
        next("FREQ=DAILY;INTERVAL=2", 0, 3 * MILLIS_PER_DAY),
        Some(4 * MILLIS_PER_DAY)
    );
}

#[test]
fn an_instant_before_the_anchor_yields_the_anchor() {
    // `after` precedes dtstart, so the anchor itself is the next occurrence.
    assert_eq!(
        next("FREQ=DAILY", 5 * MILLIS_PER_DAY, 0),
        Some(5 * MILLIS_PER_DAY)
    );
}

#[test]
fn monthly_recurrence_uses_calendar_arithmetic() {
    // 15 Jan, monthly: the next after 1 Feb is 15 Feb.
    assert_eq!(
        next("FREQ=MONTHLY", day("2026-01-15"), day("2026-02-01")),
        Some(day("2026-02-15"))
    );
    // Day-of-month clamps where the target month is shorter: 31 Jan + 1 month → 28 Feb (2026 is
    // not a leap year).
    assert_eq!(
        next("FREQ=MONTHLY", day("2026-01-31"), day("2026-02-01")),
        Some(day("2026-02-28"))
    );
}

#[test]
fn yearly_recurrence_advances_by_whole_years() {
    assert_eq!(
        next("FREQ=YEARLY", day("2024-06-10"), day("2025-01-01")),
        Some(day("2025-06-10"))
    );
}

#[test]
fn a_malformed_or_unsupported_rule_never_fires() {
    // Unsupported frequency, missing FREQ, bad interval, and empty all yield None.
    assert_eq!(next("FREQ=HOURLY", 0, MILLIS_PER_DAY), None);
    assert_eq!(next("INTERVAL=2", 0, MILLIS_PER_DAY), None);
    assert_eq!(next("FREQ=WEEKLY;INTERVAL=0", 0, MILLIS_PER_DAY), None);
    assert_eq!(next("", 0, MILLIS_PER_DAY), None);
    // An uninterpreted key (BYDAY) is ignored, falling back to the FREQ/INTERVAL.
    assert_eq!(next("FREQ=WEEKLY;BYDAY=MO", 0, 0), Some(MILLIS_PER_WEEK));
}

#[test]
fn supported_judges_which_rules_can_arm_a_wake_up() {
    // Anything next_occurrence can interpret — the four frequencies, an interval, an uninterpreted
    // BYDAY — is supported, and arms a wake-up rather than being dropped at extraction.
    assert!(supported("FREQ=WEEKLY;BYDAY=MO"));
    assert!(supported("FREQ=DAILY"));
    assert!(supported("freq=monthly;interval=3"));
    assert!(supported("FREQ=YEARLY"));
    // A free-phrased cadence the model emits in place of an rrule is not, and is rejected at
    // extraction so it never becomes a silent dud.
    assert!(!supported("every Monday"));
    assert!(!supported("FREQ=HOURLY"));
    assert!(!supported(""));
}

/// `rrule_is_supported` on a literal rule string, for brevity.
fn supported(rule: &str) -> bool {
    rrule_is_supported(&Rrule(rule.into()))
}

#[test]
fn millis_per_day_is_the_obvious_product() {
    assert_eq!(MILLIS_PER_DAY, 86_400_000);
}

#[test]
fn civil_date_validates_and_converts() {
    // 2026-06-03 is 20_607 days after the epoch.
    assert_eq!(
        civil_date_to_millis("2026-06-03"),
        Some(20_607 * MILLIS_PER_DAY)
    );
    // 2026 is not a leap year, so Feb 29 is rejected rather than rolling into March.
    assert_eq!(civil_date_to_millis("2026-02-29"), None);
    assert_eq!(civil_date_to_millis("nonsense"), None);
}

#[test]
fn day_window_spans_the_civil_day() {
    let midnight = 20_607 * MILLIS_PER_DAY;
    assert_eq!(
        day_window("2026-06-03"),
        Some((midnight, midnight + MILLIS_PER_DAY - 1))
    );
}

#[test]
fn duration_parses_days_and_weeks() {
    assert_eq!(parse_duration_millis("7 days"), Some(7 * MILLIS_PER_DAY));
    assert_eq!(parse_duration_millis("1 day"), Some(MILLIS_PER_DAY));
    assert_eq!(parse_duration_millis("2 weeks"), Some(14 * MILLIS_PER_DAY));
    assert_eq!(parse_duration_millis("soon"), None);
    assert_eq!(parse_duration_millis("3 fortnights"), None);
    assert_eq!(parse_duration_millis("-1 days"), None);
}
