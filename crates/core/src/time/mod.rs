//! Time across the crate (spec §Time). The millisecond/day constants (defined as a product so the
//! derivation is plain), civil-date conversion, the small calendar-argument parsers, and timestamp
//! formatting live here; the [`temporal`] submodule holds [`TemporalRef`], the typed occurrence value
//! an entry carries in `occurred_at`, and its denormalization. Centralized so the date logic lives in
//! one place rather than being re-derived per module.
//!
//! The constants and [`temporal`]'s bounds derivation are pure arithmetic; the civil-date
//! conversion, datetime parsing, formatting, and recurrence-instance computation
//! ([`next_occurrence`]) are backed by `jiff`.

pub mod temporal;

pub use temporal::{
    BEFORE_AFTER_EPSILON_MILLIS, CivilDate, Direction, OccurrenceBounds, Rrule, TemporalRef,
};

use std::ops::{Deref, DerefMut};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Wall-clock time as an instant, UTC. A denormalized convenience for human-readable queries and
/// recency math; `Seq` is the authoritative timeline, and `Seq` breaks ties (see spec §Time →
/// sequence vs wall-clock). Wraps [`jiff::Timestamp`] so call sites reach jiff's full API (`.to_zoned`,
/// span arithmetic, comparison) through [`Deref`], while [`Serialize`]/[`Deserialize`] and the ts-rs
/// export keep the wire shape a bare epoch-millisecond `i64`, exactly as the pre-jiff newtype did —
/// the event log and the console wire depend on that shape staying unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Timestamp(#[cfg_attr(feature = "ts", ts(type = "number"))] jiff::Timestamp);

impl Timestamp {
    /// Wrap an epoch-millisecond value trusted to be within jiff's representable range (year -9999
    /// to 9999). Panics on an out-of-range input, so this is for trusted internal arithmetic (clock
    /// advances, day-window math) — route untrusted input (deserialized wire values, Lua temporal
    /// arguments, SQLite rows) through [`Timestamp::try_from_millis`] instead.
    pub fn from_millis(millis: i64) -> Timestamp {
        Timestamp::try_from_millis(millis).unwrap_or_else(|| {
            panic!(
                "timestamp: {millis} milliseconds since the Unix epoch is outside jiff's \
                 representable range"
            )
        })
    }

    /// Wrap an epoch-millisecond value, or `None` if it is outside jiff's representable range — the
    /// fallible constructor for untrusted input.
    pub fn try_from_millis(millis: i64) -> Option<Timestamp> {
        jiff::Timestamp::from_millisecond(millis)
            .ok()
            .map(Timestamp)
    }

    /// A `const`-evaluable constructor from whole seconds since the Unix epoch, for a `const`
    /// fixture (a test's named reference instant) that needs a compile-time value rather than a
    /// runtime call. Panics (at compile time, for a `const` binding) when `seconds` is outside jiff's
    /// representable range — see [`jiff::Timestamp::constant`].
    pub const fn from_epoch_seconds(seconds: i64) -> Timestamp {
        Timestamp(jiff::Timestamp::constant(seconds, 0))
    }
}

impl Deref for Timestamp {
    type Target = jiff::Timestamp;

    fn deref(&self) -> &jiff::Timestamp {
        &self.0
    }
}

impl DerefMut for Timestamp {
    fn deref_mut(&mut self) -> &mut jiff::Timestamp {
        &mut self.0
    }
}

impl From<jiff::Timestamp> for Timestamp {
    fn from(value: jiff::Timestamp) -> Timestamp {
        Timestamp(value)
    }
}

impl From<Timestamp> for jiff::Timestamp {
    fn from(value: Timestamp) -> jiff::Timestamp {
        value.0
    }
}

impl Serialize for Timestamp {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        jiff::fmt::serde::timestamp::millisecond::required::serialize(&self.0, serializer)
    }
}

impl<'de> Deserialize<'de> for Timestamp {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Timestamp, D::Error> {
        jiff::fmt::serde::timestamp::millisecond::required::deserialize(deserializer).map(Timestamp)
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
/// calendar date. jiff parses and validates (rejecting e.g. a non-leap Feb 29, an unpadded
/// `2026-6-8`, or a year outside ±9999) so a malformed value never silently rolls over into a
/// neighboring month.
pub fn civil_date_to_millis(date: &str) -> Option<i64> {
    let date: jiff::civil::Date = date.parse().ok()?;
    let zoned = date.to_zoned(jiff::tz::TimeZone::UTC).ok()?;
    Some(zoned.timestamp().as_millisecond())
}

/// The `[midnight, end-of-day]` millisecond window of a `YYYY-MM-DD` civil day, or `None` if it does
/// not parse — the span `calendar.on` queries.
pub fn day_window(date: &str) -> Option<(i64, i64)> {
    let midnight = civil_date_to_millis(date)?;
    Some((midnight, midnight + MILLIS_PER_DAY - 1))
}

/// Parse a small calendar duration (`"7 days"`, `"2 weeks"`, `"6 months"`, singular accepted) to
/// milliseconds, or `None` if it is not `<non-negative integer> day(s)|week(s)|month(s)`. A month is
/// thirty days: these durations bound fuzzy windows (how far a calendar query looks), not civil-date
/// arithmetic, so a fixed width serves better than month-length pedantry. Deliberately narrow beyond
/// that; richer durations can follow if the agent needs them.
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
        "month" | "months" => 30 * MILLIS_PER_DAY,
        _ => return None,
    };
    count.checked_mul(per_unit)
}

/// Epoch milliseconds of either a `YYYY-MM-DD` day (taken at midnight) or an ISO 8601 datetime
/// (e.g. `2026-06-02T09:30:00Z`), or `None` — the "either form" seam the temporal extractor maps
/// model-emitted date strings through.
pub fn date_or_datetime_to_millis(text: &str) -> Option<i64> {
    let text = text.trim();
    civil_date_to_millis(text).or_else(|| {
        text.parse::<jiff::Timestamp>()
            .ok()
            .map(|timestamp| timestamp.as_millisecond())
    })
}

/// Render a timestamp as a human-readable UTC datetime (e.g. `Thursday, 01 January 1970, 00:00 UTC`).
/// Declared at conversation start and used as the reference for resolving relative phrases in
/// extraction.
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

/// Render a timestamp as an ISO 8601 / RFC 3339 instant in the operator's local timezone, offset and
/// all (e.g. `2026-06-08T16:36:22+02:00`) — the machine-sortable, second-resolution form for
/// diagnostic output on the operator's own machine, where local wall-clock reads more naturally than
/// UTC. Distinct from the UTC helpers above, which anchor the agent's own reasoning; this one is
/// purely for operator display.
pub fn format_iso8601(at: Timestamp) -> String {
    at.to_zoned(jiff::tz::TimeZone::system())
        .strftime("%Y-%m-%dT%H:%M:%S%:z")
        .to_string()
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
    at.to_zoned(jiff::tz::TimeZone::UTC)
        .strftime(format)
        .to_string()
}
mod recurrence;

use recurrence::add_calendar;
pub use recurrence::{
    Freq, MAX_RECURRENCE_STEPS, next_occurrence, parse_rrule, rrule_is_supported,
};

#[cfg(test)]
mod tests;
