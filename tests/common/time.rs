//! Centralised timestamps for the integration-test suite, named and dated so a test reads as intent
//! rather than a bare millisecond count. Use these instead of `Timestamp::from_millis(<magic>)`.

use zuihitsu::Timestamp;

/// Midnight UTC, 8 June 2026 — the present-day "now" the suite anchors to, stated as a civil date
/// rather than an epoch count. A lifelike, non-epoch base: the model resolves relative phrases
/// ("last Tuesday") against a realistic date, and stamped turns read as the present rather than
/// 1970.
pub fn test_now() -> Timestamp {
    jiff::civil::date(2026, 6, 8)
        .to_zoned(jiff::tz::TimeZone::UTC)
        .expect("midnight UTC of a valid civil date is always representable")
        .timestamp()
        .into()
}

/// 1970-01-01 00:00:01 UTC — an early reference instant for tests where only the *ordering* of writes
/// matters, not the wall-clock value. A deterministic, far-from-now baseline.
pub const EARLY: Timestamp = Timestamp::from_epoch_seconds(1);
