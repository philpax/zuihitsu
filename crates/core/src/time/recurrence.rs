use super::{MILLIS_PER_DAY, MILLIS_PER_WEEK, Timestamp, temporal::Rrule};

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
pub enum Freq {
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

/// The scan bound for month/year recurrences, guarding against a pathological far-past anchor. A
/// thousand years of monthly steps is far beyond any real reminder; past it, `next_occurrence` returns
/// `None` rather than looping.
pub const MAX_RECURRENCE_STEPS: i64 = 12_000;

/// Parse the supported subset of an rrule into `(freq, interval)`. Keys are `;`-separated `KEY=VALUE`
/// pairs, case-insensitive on keys and on the `FREQ` value; `INTERVAL` defaults to 1 and must be a
/// positive integer. Returns `None` if `FREQ` is absent, unsupported, or `INTERVAL` is malformed.
pub fn parse_rrule(rule: &str) -> Option<(Freq, i64)> {
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
pub(super) fn add_calendar(from: Timestamp, months: i64, years: i64) -> Option<Timestamp> {
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
pub(super) fn parse_ymd(text: &str) -> Option<(i64, u32, u32)> {
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
pub(super) fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month = i64::from(month);
    let day = i64::from(day);
    let day_of_year = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}
