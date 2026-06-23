//! Time across the crate (spec §Time). The millisecond/day constants (defined as a product so the
//! derivation is plain), civil-date conversion, the small calendar-argument parsers, and timestamp
//! formatting live here; the [`temporal`] submodule holds [`TemporalRef`], the typed occurrence value
//! an entry carries in `occurred_at`, and its denormalization. Centralized so the date logic lives in
//! one place rather than being re-derived per module.
//!
//! The constants, pure civil-date math, and [`temporal`] are dependency-free; the datetime parsing,
//! formatting, and recurrence-instance computation ([`next_occurrence`]) are backed by `jiff`.

pub mod temporal;

pub use temporal::{
    BEFORE_AFTER_EPSILON_MILLIS, CivilDate, Direction, OccurrenceBounds, Rrule, TemporalRef,
};

use serde::{Deserialize, Serialize};

/// Wall-clock time as milliseconds since the Unix epoch, UTC. A denormalized convenience for
/// human-readable queries and recency math; `Seq` is the authoritative timeline, and `Seq` breaks
/// ties (see spec §Time → sequence vs wall-clock).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Timestamp(#[cfg_attr(feature = "ts", ts(type = "number"))] pub i64);

impl Timestamp {
    pub fn from_millis(millis: i64) -> Timestamp {
        Timestamp(millis)
    }

    pub fn as_millis(self) -> i64 {
        self.0
    }
}

pub const MILLIS_PER_SECOND: i64 = 1_000;
pub const SECONDS_PER_MINUTE: i64 = 60;
pub const MINUTES_PER_HOUR: i64 = 60;
pub const HOURS_PER_DAY: i64 = 24;
pub const DAYS_PER_WEEK: i64 = 7;

pub const MILLIS_PER_MINUTE: i64 = MILLIS_PER_SECOND * SECONDS_PER_MINUTE;
pub const MILLIS_PER_HOUR: i64 = MILLIS_PER_MINUTE * MINUTES_PER_HOUR;
pub const MILLIS_PER_DAY: i64 = MILLIS_PER_HOUR * HOURS_PER_DAY;
pub const MILLIS_PER_WEEK: i64 = MILLIS_PER_DAY * DAYS_PER_WEEK;

/// Midnight UTC of a `YYYY-MM-DD` civil day as epoch milliseconds, or `None` if it is not a valid
/// calendar date. Validates the date (rejecting e.g. a non-leap Feb 29) so a malformed value never
/// silently rolls over into a neighboring month.
pub fn civil_date_to_millis(date: &str) -> Option<i64> {
    let (year, month, day) = parse_ymd(date)?;
    Some(days_from_civil(year, month, day) * MILLIS_PER_DAY)
}

/// The `[midnight, end-of-day]` millisecond window of a `YYYY-MM-DD` civil day, or `None` if it does
/// not parse — the span `calendar.on` queries.
pub fn day_window(date: &str) -> Option<(i64, i64)> {
    let midnight = civil_date_to_millis(date)?;
    Some((midnight, midnight + MILLIS_PER_DAY - 1))
}

/// Parse a small calendar duration (`"7 days"`, `"2 weeks"`, singular accepted) to milliseconds, or
/// `None` if it is not `<non-negative integer> day(s)|week(s)`. Deliberately narrow; richer durations
/// can follow if the agent needs them.
pub fn parse_duration_millis(text: &str) -> Option<i64> {
    let mut parts = text.split_whitespace();
    let count: i64 = parts.next()?.parse().ok()?;
    let unit = parts.next()?;
    if parts.next().is_some() || count < 0 {
        return None;
    }
    let per_unit = match unit {
        "day" | "days" => MILLIS_PER_DAY,
        "week" | "weeks" => MILLIS_PER_WEEK,
        _ => return None,
    };
    count.checked_mul(per_unit)
}

/// Epoch milliseconds of an ISO 8601 datetime (e.g. `2026-06-02T00:00:00Z`), or `None` if it does not
/// parse.
pub fn datetime_to_millis(text: &str) -> Option<i64> {
    text.trim()
        .parse::<jiff::Timestamp>()
        .ok()
        .map(|timestamp| timestamp.as_millisecond())
}

/// Epoch milliseconds of either a `YYYY-MM-DD` day (taken at midnight) or an ISO datetime, or `None`.
pub fn date_or_datetime_to_millis(text: &str) -> Option<i64> {
    civil_date_to_millis(text.trim()).or_else(|| datetime_to_millis(text))
}

/// Render a timestamp as a human-readable UTC datetime (e.g. `Thursday, 01 January 1970, 00:00 UTC`),
/// falling back to raw epoch milliseconds for a time outside the supported range. Declared at
/// conversation start and used as the reference for resolving relative phrases in extraction.
pub fn format_datetime(at: Timestamp) -> String {
    format_with(at, "%A, %d %B %Y, %H:%M UTC")
}

/// A compact wall-clock stamp for prefixing a replayed turn (spec §Time → "Now"): `Mon 2026-06-08
/// 14:36 UTC`. Briefer than [`format_datetime`], which anchors the session start in prose, because it
/// rides on every buffered turn. Carries the weekday so the agent can resolve a relative date ("this
/// Friday", "next Tuesday") against the message's own stamp without computing the weekday from the
/// bare date — an error-prone step that mis-scheduled one-off reminders by a day.
pub fn format_stamp(at: Timestamp) -> String {
    format_with(at, "%a %Y-%m-%d %H:%M UTC")
}

/// Render a timestamp as a concise UTC day (e.g. `Wed 03 Jun`) — the `<upcoming/>` brief shape.
pub fn format_day(at: Timestamp) -> String {
    format_with(at, "%a %d %b")
}

/// Render an entry's [`TemporalRef`] occurrence as a compact, human-readable phrase for a read —
/// `2027-03-15` for a day, the instant for a precise time, a span for a range, and the rule or anchor
/// for the vaguer forms. So a dated fact shows *when* it happens on read, rather than hiding the date
/// in a structured field the agent has to inspect (or search for) separately.
pub fn format_occurrence(occurred_at: &TemporalRef) -> String {
    match occurred_at {
        TemporalRef::Day(date) => date.0.to_string(),
        TemporalRef::Instant(at) => format_with(*at, "%Y-%m-%d %H:%M UTC"),
        TemporalRef::Range { start, end } => format!(
            "{} – {}",
            format_with(*start, "%Y-%m-%d"),
            format_with(*end, "%Y-%m-%d")
        ),
        TemporalRef::Approx { center, fuzz_days } => {
            format!(
                "around {} (±{fuzz_days}d)",
                format_with(*center, "%Y-%m-%d")
            )
        }
        TemporalRef::Recurring(rule) => format!("recurring: {}", rule.0),
        TemporalRef::BeforeAfter { dir, anchor } => {
            let side = match dir {
                Direction::Before => "before",
                Direction::After => "after",
            };
            format!("{side} {}", anchor.as_str())
        }
    }
}

/// Today's civil day (`YYYY-MM-DD`, UTC) for `now` — the anchor the relative-date constructors build
/// on, so the agent names an operation ("next Friday", "in two weeks") instead of computing the date.
pub fn today(now: Timestamp) -> String {
    format_with(now, "%Y-%m-%d")
}

/// A civil day shifted by `days` (negative goes back), or `None` if `date` is not a valid `YYYY-MM-DD`.
/// Exact: a UTC day plus whole days has no DST hazard.
pub fn add_days(date: &str, days: i64) -> Option<String> {
    let millis = civil_date_to_millis(date)?;
    let shifted = millis.checked_add(days.checked_mul(MILLIS_PER_DAY)?)?;
    Some(format_with(Timestamp::from_millis(shifted), "%Y-%m-%d"))
}

/// A civil day shifted by `months`, day-of-month preserved where it exists and clamped where it does
/// not (31 Jan + 1 month → 28/29 Feb), or `None` if `date` is invalid or the result is out of range.
pub fn add_months(date: &str, months: i64) -> Option<String> {
    let millis = civil_date_to_millis(date)?;
    let shifted = add_calendar(Timestamp::from_millis(millis), months, 0)?;
    Some(format_with(shifted, "%Y-%m-%d"))
}

/// The full weekday name of a civil day (e.g. `Monday`), or `None` if `date` is not a valid date.
pub fn weekday(date: &str) -> Option<String> {
    let millis = civil_date_to_millis(date)?;
    Some(format_with(Timestamp::from_millis(millis), "%A"))
}

/// The soonest civil day on or after `now`'s day whose weekday is `name` (a case-insensitive full
/// weekday name), or `None` if `name` is not a weekday — `today` itself when today already matches. So
/// "this Friday" is a lookup, never arithmetic the model carries in its head.
pub fn next_weekday(now: Timestamp, name: &str) -> Option<String> {
    const WEEKDAYS: [&str; 7] = [
        "monday",
        "tuesday",
        "wednesday",
        "thursday",
        "friday",
        "saturday",
        "sunday",
    ];
    let target = name.trim().to_ascii_lowercase();
    if !WEEKDAYS.contains(&target.as_str()) {
        return None;
    }
    let today = today(now);
    (0..7).find_map(|offset| {
        let day = add_days(&today, offset)?;
        (weekday(&day)?.to_ascii_lowercase() == target).then_some(day)
    })
}

fn format_with(at: Timestamp, format: &str) -> String {
    match jiff::Timestamp::from_millisecond(at.as_millis()) {
        Ok(timestamp) => timestamp
            .to_zoned(jiff::tz::TimeZone::UTC)
            .strftime(format)
            .to_string(),
        Err(_) => format!("{} milliseconds since the Unix epoch", at.as_millis()),
    }
}

/// The first instance of a recurrence strictly after `after`, anchored at `dtstart` — the entry's
/// assertion time, since the rrule string carries no `DTSTART` (spec §Time, §Recurring materialization
/// and wake-up arming). Occurrences are `dtstart`, `dtstart + interval`, `dtstart + 2·interval`, …;
/// this returns the earliest one past `after`, the next instance the wake-up scheduler arms.
///
/// Interprets a deliberately narrow subset of RFC 5545: `FREQ` (`DAILY`, `WEEKLY`, `MONTHLY`,
/// `YEARLY`) and `INTERVAL` (default 1). `BYDAY`/`COUNT`/`UNTIL` and the rest are not interpreted; a
/// rule that omits or misuses `FREQ`, or names an unsupported frequency, yields `None` so a malformed
/// rule simply never fires rather than erroring. Day/week steps are exact milliseconds; month/year
/// steps use calendar arithmetic (so 31 Jan + 1 month is 28/29 Feb), bounded by [`MAX_RECURRENCE_STEPS`]
/// against a pathological far-past anchor.
pub fn next_occurrence(rule: &Rrule, dtstart: Timestamp, after: Timestamp) -> Option<Timestamp> {
    let (freq, interval) = parse_rrule(rule.0.as_str())?;
    // The k-th occurrence (k ≥ 0), or None on arithmetic overflow.
    let occurrence = |k: i64| -> Option<Timestamp> {
        let steps = k.checked_mul(interval)?;
        match freq {
            Freq::Daily => add_millis(dtstart, steps.checked_mul(MILLIS_PER_DAY)?),
            Freq::Weekly => add_millis(dtstart, steps.checked_mul(MILLIS_PER_WEEK)?),
            Freq::Monthly => add_calendar(dtstart, steps, 0),
            Freq::Yearly => add_calendar(dtstart, 0, steps),
        }
    };
    // The 0th occurrence is dtstart itself; if it is already past `after`, it is the next one.
    if dtstart > after {
        return occurrence(0);
    }
    // Day/week steps are uniform, so the index is a direct division; month/year steps vary in length,
    // so scan forward from a lower-bound estimate. Either way, find the first occurrence past `after`.
    let elapsed = after.as_millis().checked_sub(dtstart.as_millis())?;
    let start = match freq {
        Freq::Daily => elapsed / (interval * MILLIS_PER_DAY) + 1,
        Freq::Weekly => elapsed / (interval * MILLIS_PER_WEEK) + 1,
        // ~28-day lower bound never overshoots, so the scan lands in a few steps.
        Freq::Monthly => elapsed / (interval * 28 * MILLIS_PER_DAY) + 1,
        Freq::Yearly => elapsed / (interval * 365 * MILLIS_PER_DAY) + 1,
    };
    let first = start.max(1);
    for k in first..first.saturating_add(MAX_RECURRENCE_STEPS) {
        let candidate = occurrence(k)?;
        if candidate > after {
            return Some(candidate);
        }
    }
    None
}

/// Whether a recurrence rule is one this build can interpret: it parses to a supported `FREQ` with a
/// well-formed `INTERVAL`, so [`next_occurrence`] can arm a wake-up from it. The extractor rejects a
/// rule that fails this (a model free-phrasing such as "every Monday") rather than committing it as a
/// [`TemporalRef::Recurring`] that parses here as `None` and so silently never fires — a dud entry
/// whose schedule no one can derive.
pub fn rrule_is_supported(rule: &Rrule) -> bool {
    parse_rrule(rule.0.as_str()).is_some()
}

/// The recurrence frequency this build interprets — the `FREQ` values of the supported subset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Freq {
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

/// The scan bound for month/year recurrences, guarding against a pathological far-past anchor. A
/// thousand years of monthly steps is far beyond any real reminder; past it, `next_occurrence` returns
/// `None` rather than looping.
const MAX_RECURRENCE_STEPS: i64 = 12_000;

/// Parse the supported subset of an rrule into `(freq, interval)`. Keys are `;`-separated `KEY=VALUE`
/// pairs, case-insensitive on keys and on the `FREQ` value; `INTERVAL` defaults to 1 and must be a
/// positive integer. Returns `None` if `FREQ` is absent, unsupported, or `INTERVAL` is malformed.
fn parse_rrule(rule: &str) -> Option<(Freq, i64)> {
    let mut freq = None;
    let mut interval = 1i64;
    for part in rule.split(';') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let (key, value) = part.split_once('=')?;
        match key.trim().to_ascii_uppercase().as_str() {
            "FREQ" => {
                freq = Some(match value.trim().to_ascii_uppercase().as_str() {
                    "DAILY" => Freq::Daily,
                    "WEEKLY" => Freq::Weekly,
                    "MONTHLY" => Freq::Monthly,
                    "YEARLY" => Freq::Yearly,
                    _ => return None,
                });
            }
            "INTERVAL" => {
                interval = value.trim().parse().ok().filter(|n| *n >= 1)?;
            }
            // Other keys (BYDAY, COUNT, UNTIL, …) are not interpreted in this subset.
            _ => {}
        }
    }
    Some((freq?, interval))
}

/// `from` shifted by `delta` milliseconds, or `None` on overflow.
fn add_millis(from: Timestamp, delta: i64) -> Option<Timestamp> {
    from.as_millis()
        .checked_add(delta)
        .map(Timestamp::from_millis)
}

/// `from` shifted by `months` months and `years` years using calendar arithmetic (via `jiff`), so a
/// month step lands on the same day-of-month where it exists and clamps where it does not (31 Jan +
/// 1 month → 28/29 Feb). `None` if the timestamp or the shifted result falls outside the supported
/// range.
fn add_calendar(from: Timestamp, months: i64, years: i64) -> Option<Timestamp> {
    let zoned = jiff::Timestamp::from_millisecond(from.as_millis())
        .ok()?
        .to_zoned(jiff::tz::TimeZone::UTC);
    let span = jiff::Span::new()
        .try_months(months)
        .ok()?
        .try_years(years)
        .ok()?;
    let shifted = zoned.checked_add(span).ok()?;
    Some(Timestamp::from_millis(shifted.timestamp().as_millisecond()))
}

/// Parse `YYYY-MM-DD` into a validated `(year, month, day)`, rejecting impossible dates (bad month, or
/// a day past the month's length, leap years included).
fn parse_ymd(text: &str) -> Option<(i64, u32, u32)> {
    let mut parts = text.split('-');
    let year: i64 = parts.next()?.parse().ok()?;
    let month: u32 = parts.next()?.parse().ok()?;
    let day: u32 = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    if !(1..=12).contains(&month) || day < 1 || day > days_in_month(year, month) {
        return None;
    }
    Some((year, month, day))
}

fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

fn is_leap_year(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

/// Days since the Unix epoch (1970-01-01) for a civil date, via Howard Hinnant's `days_from_civil`
/// algorithm — exact for the proleptic Gregorian calendar with no date-crate dependency.
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month = i64::from(month);
    let day = i64::from(day);
    let day_of_year = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

#[cfg(test)]
mod tests {
    use super::{
        MILLIS_PER_DAY, MILLIS_PER_WEEK, Rrule, Timestamp, add_days, add_months,
        civil_date_to_millis, day_window, next_occurrence, next_weekday, parse_duration_millis,
        rrule_is_supported, today, weekday,
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
}
