//! Time across the crate (spec §Time). The millisecond/day constants (defined as a product so the
//! derivation is plain), civil-date conversion, the small calendar-argument parsers, and timestamp
//! formatting live here; the [`temporal`] submodule holds [`TemporalRef`], the typed occurrence value
//! an entry carries in `occurred_at`, and its denormalization. Centralized so the date logic lives in
//! one place rather than being re-derived per module.
//!
//! The constants, pure civil-date math, and [`temporal`] are always available; the `jiff`-backed
//! datetime parsing and formatting are gated on `sqlite` (which is what pulls `jiff`), since the
//! no-I/O build never formats or parses datetimes.

pub mod temporal;

pub use temporal::{
    BEFORE_AFTER_EPSILON_MILLIS, CivilDate, Direction, OccurrenceBounds, Rrule, TemporalRef,
};

use serde::{Deserialize, Serialize};

/// Wall-clock time as milliseconds since the Unix epoch, UTC. A denormalized convenience for
/// human-readable queries and recency math; `Seq` is the authoritative timeline, and `Seq` breaks
/// ties (see spec §Time → sequence vs wall-clock).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Timestamp(pub i64);

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
#[cfg(feature = "sqlite")]
pub fn datetime_to_millis(text: &str) -> Option<i64> {
    text.trim()
        .parse::<jiff::Timestamp>()
        .ok()
        .map(|timestamp| timestamp.as_millisecond())
}

/// Epoch milliseconds of either a `YYYY-MM-DD` day (taken at midnight) or an ISO datetime, or `None`.
#[cfg(feature = "sqlite")]
pub fn date_or_datetime_to_millis(text: &str) -> Option<i64> {
    civil_date_to_millis(text.trim()).or_else(|| datetime_to_millis(text))
}

/// Render a timestamp as a human-readable UTC datetime (e.g. `Thursday, 01 January 1970, 00:00 UTC`),
/// falling back to raw epoch milliseconds for a time outside the supported range. Declared at
/// conversation start and used as the reference for resolving relative phrases in extraction.
#[cfg(feature = "sqlite")]
pub fn format_datetime(at: Timestamp) -> String {
    format_with(at, "%A, %d %B %Y, %H:%M UTC")
}

/// Render a timestamp as a concise UTC day (e.g. `Wed 03 Jun`) — the `<upcoming/>` brief shape.
#[cfg(feature = "sqlite")]
pub fn format_day(at: Timestamp) -> String {
    format_with(at, "%a %d %b")
}

#[cfg(feature = "sqlite")]
fn format_with(at: Timestamp, format: &str) -> String {
    match jiff::Timestamp::from_millisecond(at.as_millis()) {
        Ok(timestamp) => timestamp
            .to_zoned(jiff::tz::TimeZone::UTC)
            .strftime(format)
            .to_string(),
        Err(_) => format!("{} milliseconds since the Unix epoch", at.as_millis()),
    }
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
    use super::{MILLIS_PER_DAY, civil_date_to_millis, day_window, parse_duration_millis};

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
