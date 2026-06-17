//! Staleness: when a fast-changing fact has aged past usefulness (spec §Recency and volatility).
//!
//! This is distinct from the recency *ranking* decay in search, which gently de-weights every memory
//! by age over its volatility-scaled `tau` (so an old fact merely ranks lower). Staleness is a binary
//! legibility signal that fires only for a memory the agent deliberately marked `High` volatility —
//! one that holds fast-changing facts like a current location or a status. Once such an entry ages
//! past the staleness horizon it reads with a `[stale]` marker, so the agent surfaces it as possibly
//! out of date ("last I heard …") rather than asserting it as current. `Medium` (the default) and
//! `Low` never go stale, so the marker is opt-in and never a false alarm on a durable fact.

use crate::{
    event::Volatility,
    time::{self, Timestamp},
};

/// The inline marker an aged `High`-volatility entry carries when surfaced, so the agent hedges rather
/// than asserting a fading fact as current. Rendered like the other entry markers (`[disputed]`,
/// `[via …]`), appended to the fact on every live surface.
pub const STALE_MARKER: &str = "[stale]";

/// Whether an entry is stale: its memory is `High` volatility and its effective time — its occurrence
/// if dated, else when it was asserted — is older than the staleness horizon. Only `High` ever goes
/// stale; the default `Medium` and durable `Low` never do, which keeps the marker opt-in.
pub fn is_stale(volatility: Volatility, effective_time: Timestamp, now: Timestamp) -> bool {
    if volatility != Volatility::High {
        return false;
    }
    let age_days =
        (now.as_millis() - effective_time.as_millis()).max(0) as f64 / time::MILLIS_PER_DAY as f64;
    age_days > STALE_HIGH_DAYS
}

/// The staleness horizon for a `High`-volatility memory, in days: an entry older than this reads as
/// stale. `High` is the agent's own signal that the fact dates quickly (a current role, project,
/// location, or status), so a month-old one is worth hedging; it sits inside the `High` ranking `tau`
/// (90 days), so a fact reads stale while still ranking relevant enough to surface.
const STALE_HIGH_DAYS: f64 = 30.0;

#[cfg(test)]
mod tests {
    use super::{STALE_HIGH_DAYS, is_stale};
    use crate::{event::Volatility, time::Timestamp};

    fn days(n: f64) -> Timestamp {
        Timestamp::from_millis((n * 86_400_000.0) as i64)
    }

    #[test]
    fn only_high_volatility_goes_stale() {
        let now = days(STALE_HIGH_DAYS + 10.0);
        let old = days(0.0);
        // A long-aged entry: stale only when the memory is High volatility.
        assert!(is_stale(Volatility::High, old, now));
        assert!(!is_stale(Volatility::Medium, old, now));
        assert!(!is_stale(Volatility::Low, old, now));
    }

    #[test]
    fn high_volatility_is_fresh_within_the_horizon_and_stale_past_it() {
        let old = days(0.0);
        // Inside the horizon: fresh. Past it: stale.
        assert!(!is_stale(
            Volatility::High,
            old,
            days(STALE_HIGH_DAYS - 1.0)
        ));
        assert!(is_stale(Volatility::High, old, days(STALE_HIGH_DAYS + 1.0)));
    }
}
