//! Bi-temporal occurrence references: *when a recorded fact is about*, as distinct from when it was
//! recorded (`asserted_at`). See spec §Time. A [`TemporalRef`] is the typed value an entry carries
//! in `occurred_at`; the materializer denormalizes it into three sortable/queryable columns via
//! [`TemporalRef::bounds`] for recency ranking and (later) calendar windows.
//!
//! Deliberately pure and dependency-free — no graph, no date crate (it borrows the parent [`super`]
//! module's civil-date math) — because the event layer that carries `occurred_at` stays free of
//! storage and graph dependencies. `BeforeAfter` anchor resolution is a graph read, so the materializer
//! performs it and passes the resolved anchor bounds *into* [`TemporalRef::bounds`] rather than this
//! module reaching into the graph.

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use super::{MILLIS_PER_DAY, MILLIS_PER_HOUR, Timestamp, civil_date_to_millis};
use crate::ids::MemoryName;

/// The nominal shift a [`TemporalRef::BeforeAfter`] applies to its anchor's representative instant —
/// a tuning knob, like the recency `τ` constants (spec §Time). One hour.
pub const BEFORE_AFTER_EPSILON_MILLIS: i64 = MILLIS_PER_HOUR;

/// A typed, vague-capable reference to when a fact occurred (spec §Time → bi-temporality). Stored as
/// tagged JSON in the `occurred_at` column; the materializer derives `occurred_sort`/`occurred_lo`/
/// `occurred_hi` from it via [`TemporalRef::bounds`].
///
/// The agent writes it as a tagged Lua table whose single key names the variant:
/// `{ instant = <ms> }`, `{ day = "YYYY-MM-DD" }`, `{ range = { start = <ms>, end = <ms> } }`,
/// `{ approx = { center = <ms>, fuzz_days = <n> } }`, `{ recurring = "<rrule>" }`, or
/// `{ before_after = { dir = "before" | "after", anchor = "event/..." } }`. Natural-language phrases
/// ("last week") are resolved to this type by a later increment's extraction pass, not here.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TemporalRef {
    /// A precise instant.
    Instant(Timestamp),
    /// A calendar day (no time of day).
    Day(CivilDate),
    /// A closed interval between two instants.
    Range { start: Timestamp, end: Timestamp },
    /// A fuzzy point: a center plus a symmetric tolerance in days.
    Approx { center: Timestamp, fuzz_days: u32 },
    /// A recurrence rule, never expanded into discrete instances in the log (spec §Known
    /// limitations); a later increment computes virtual instances on the fly.
    Recurring(Rrule),
    /// Anchored relative to another memory's occurrence (e.g. `after event/dave-wedding`).
    BeforeAfter { dir: Direction, anchor: MemoryName },
}

/// Which side of an anchor a [`TemporalRef::BeforeAfter`] sits on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Before,
    After,
}

/// A calendar day as an ISO `YYYY-MM-DD` string. A day is not an instant: keeping the civil date
/// preserves "render it as a day" and lets the materializer derive its noon and day bounds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CivilDate(pub SmolStr);

/// An opaque RFC-5545 recurrence rule. Stored verbatim; not interpreted in this increment.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Rrule(pub SmolStr);

/// The three instants the materializer denormalizes from a [`TemporalRef`]: a representative `sort`
/// for ranking, and a `[lo, hi]` bounding interval for calendar windows. Any may be absent (e.g. a
/// `Recurring` rule, or an unresolvable `BeforeAfter` anchor).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OccurrenceBounds {
    pub sort: Option<Timestamp>,
    pub lo: Option<Timestamp>,
    pub hi: Option<Timestamp>,
}

impl TemporalRef {
    /// Derive the denormalized bounds (spec §Time → denormalized columns). Pure for every variant
    /// except [`TemporalRef::BeforeAfter`], whose `anchor` bounds the materializer resolves from the
    /// graph and passes in — `None` when the anchor is unknown or itself untimed, which yields empty
    /// bounds rather than an error (a vague or missing anchor is agent input, not a failure).
    pub fn bounds(
        &self,
        anchor: Option<OccurrenceBounds>,
        epsilon_millis: i64,
    ) -> OccurrenceBounds {
        match self {
            TemporalRef::Instant(at) => OccurrenceBounds::point(*at),
            TemporalRef::Day(date) => match date.midnight_millis() {
                Some(midnight) => OccurrenceBounds {
                    sort: Some(Timestamp::from_millis(midnight + MILLIS_PER_DAY / 2)),
                    lo: Some(Timestamp::from_millis(midnight)),
                    hi: Some(Timestamp::from_millis(midnight + MILLIS_PER_DAY - 1)),
                },
                None => OccurrenceBounds::default(),
            },
            TemporalRef::Range { start, end } => {
                let (lo, hi) = if start.as_millis() <= end.as_millis() {
                    (*start, *end)
                } else {
                    (*end, *start)
                };
                OccurrenceBounds {
                    sort: Some(Timestamp::from_millis(
                        (lo.as_millis() + hi.as_millis()) / 2,
                    )),
                    lo: Some(lo),
                    hi: Some(hi),
                }
            }
            TemporalRef::Approx { center, fuzz_days } => {
                let fuzz = i64::from(*fuzz_days) * MILLIS_PER_DAY;
                OccurrenceBounds {
                    sort: Some(*center),
                    lo: Some(Timestamp::from_millis(center.as_millis() - fuzz)),
                    hi: Some(Timestamp::from_millis(center.as_millis() + fuzz)),
                }
            }
            // No fixed instant; calendar expansion happens on the fly in a later increment.
            TemporalRef::Recurring(_) => OccurrenceBounds::default(),
            TemporalRef::BeforeAfter { dir, .. } => {
                let Some(anchor) = anchor else {
                    return OccurrenceBounds::default();
                };
                let Some(anchor_sort) = anchor.sort else {
                    return OccurrenceBounds::default();
                };
                let shift = match dir {
                    Direction::Before => -epsilon_millis,
                    Direction::After => epsilon_millis,
                };
                let sort = Timestamp::from_millis(anchor_sort.as_millis() + shift);
                // Propagate the anchor's interval when it is vague (lo != hi), shifted with it; for a
                // point anchor the shifted instant is the whole interval.
                let vague = anchor.lo != anchor.hi;
                let (lo, hi) = if vague {
                    (shifted(anchor.lo, shift), shifted(anchor.hi, shift))
                } else {
                    (Some(sort), Some(sort))
                };
                OccurrenceBounds {
                    sort: Some(sort),
                    lo,
                    hi,
                }
            }
        }
    }
}

impl OccurrenceBounds {
    fn point(at: Timestamp) -> OccurrenceBounds {
        OccurrenceBounds {
            sort: Some(at),
            lo: Some(at),
            hi: Some(at),
        }
    }
}

impl CivilDate {
    /// Midnight UTC of this civil day as epoch milliseconds, or `None` if the string is not a valid
    /// `YYYY-MM-DD` calendar date.
    pub fn midnight_millis(&self) -> Option<i64> {
        civil_date_to_millis(self.0.as_str())
    }
}

fn shifted(at: Option<Timestamp>, shift: i64) -> Option<Timestamp> {
    at.map(|at| Timestamp::from_millis(at.as_millis() + shift))
}

#[cfg(test)]
mod tests {
    use super::{
        BEFORE_AFTER_EPSILON_MILLIS, CivilDate, Direction, OccurrenceBounds, Rrule, TemporalRef,
    };
    use crate::{
        ids::MemoryName,
        time::{MILLIS_PER_DAY, Timestamp},
    };

    fn ts(millis: i64) -> Timestamp {
        Timestamp::from_millis(millis)
    }

    // 2026-06-03 is 20_607 days after the epoch; midnight UTC is that many days of millis.
    const JUNE_3_2026_MIDNIGHT: i64 = 20_607 * MILLIS_PER_DAY;

    #[test]
    fn instant_is_a_point() {
        let bounds = TemporalRef::Instant(ts(1_000)).bounds(None, BEFORE_AFTER_EPSILON_MILLIS);
        assert_eq!(
            bounds,
            OccurrenceBounds {
                sort: Some(ts(1_000)),
                lo: Some(ts(1_000)),
                hi: Some(ts(1_000)),
            }
        );
    }

    #[test]
    fn day_sorts_at_noon_and_bounds_the_day() {
        let bounds = TemporalRef::Day(CivilDate("2026-06-03".into()))
            .bounds(None, BEFORE_AFTER_EPSILON_MILLIS);
        assert_eq!(
            bounds.sort,
            Some(ts(JUNE_3_2026_MIDNIGHT + MILLIS_PER_DAY / 2))
        );
        assert_eq!(bounds.lo, Some(ts(JUNE_3_2026_MIDNIGHT)));
        assert_eq!(
            bounds.hi,
            Some(ts(JUNE_3_2026_MIDNIGHT + MILLIS_PER_DAY - 1))
        );
    }

    #[test]
    fn invalid_day_yields_empty_bounds() {
        // 2026 is not a leap year, so Feb 29 is impossible and must not roll over into March.
        let bounds = TemporalRef::Day(CivilDate("2026-02-29".into()))
            .bounds(None, BEFORE_AFTER_EPSILON_MILLIS);
        assert_eq!(bounds, OccurrenceBounds::default());
    }

    #[test]
    fn range_sorts_at_midpoint_and_normalizes_order() {
        let forward = TemporalRef::Range {
            start: ts(0),
            end: ts(100),
        }
        .bounds(None, BEFORE_AFTER_EPSILON_MILLIS);
        assert_eq!(forward.sort, Some(ts(50)));
        assert_eq!(forward.lo, Some(ts(0)));
        assert_eq!(forward.hi, Some(ts(100)));
        // A reversed range normalizes lo/hi rather than producing a negative interval.
        let reversed = TemporalRef::Range {
            start: ts(100),
            end: ts(0),
        }
        .bounds(None, BEFORE_AFTER_EPSILON_MILLIS);
        assert_eq!(reversed.lo, Some(ts(0)));
        assert_eq!(reversed.hi, Some(ts(100)));
    }

    #[test]
    fn approx_fuzzes_symmetrically_in_days() {
        let bounds = TemporalRef::Approx {
            center: ts(10 * MILLIS_PER_DAY),
            fuzz_days: 2,
        }
        .bounds(None, BEFORE_AFTER_EPSILON_MILLIS);
        assert_eq!(bounds.sort, Some(ts(10 * MILLIS_PER_DAY)));
        assert_eq!(bounds.lo, Some(ts(8 * MILLIS_PER_DAY)));
        assert_eq!(bounds.hi, Some(ts(12 * MILLIS_PER_DAY)));
    }

    #[test]
    fn recurring_has_no_fixed_instant() {
        let bounds = TemporalRef::Recurring(Rrule("FREQ=WEEKLY".into()))
            .bounds(None, BEFORE_AFTER_EPSILON_MILLIS);
        assert_eq!(bounds, OccurrenceBounds::default());
    }

    #[test]
    fn before_after_shifts_a_point_anchor() {
        let anchor = OccurrenceBounds::point(ts(1_000));
        let after = TemporalRef::BeforeAfter {
            dir: Direction::After,
            anchor: MemoryName::new("event/wedding"),
        }
        .bounds(Some(anchor), 10);
        assert_eq!(after.sort, Some(ts(1_010)));
        assert_eq!(after.lo, Some(ts(1_010)));
        assert_eq!(after.hi, Some(ts(1_010)));
        let before = TemporalRef::BeforeAfter {
            dir: Direction::Before,
            anchor: MemoryName::new("event/wedding"),
        }
        .bounds(Some(anchor), 10);
        assert_eq!(before.sort, Some(ts(990)));
    }

    #[test]
    fn before_after_propagates_a_vague_anchor_interval() {
        let vague = OccurrenceBounds {
            sort: Some(ts(1_000)),
            lo: Some(ts(900)),
            hi: Some(ts(1_100)),
        };
        let bounds = TemporalRef::BeforeAfter {
            dir: Direction::After,
            anchor: MemoryName::new("event/move"),
        }
        .bounds(Some(vague), 10);
        assert_eq!(bounds.sort, Some(ts(1_010)));
        assert_eq!(bounds.lo, Some(ts(910)));
        assert_eq!(bounds.hi, Some(ts(1_110)));
    }

    #[test]
    fn before_after_without_a_resolvable_anchor_is_empty() {
        let unresolved = TemporalRef::BeforeAfter {
            dir: Direction::After,
            anchor: MemoryName::new("event/unknown"),
        }
        .bounds(None, 10);
        assert_eq!(unresolved, OccurrenceBounds::default());
        // An anchor that resolves but has no representative instant is equally empty.
        let untimed = TemporalRef::BeforeAfter {
            dir: Direction::After,
            anchor: MemoryName::new("event/untimed"),
        }
        .bounds(Some(OccurrenceBounds::default()), 10);
        assert_eq!(untimed, OccurrenceBounds::default());
    }

    #[test]
    fn round_trips_through_tagged_json() {
        let cases = [
            (TemporalRef::Instant(ts(1_000)), "{\"instant\":1000}"),
            (
                TemporalRef::Day(CivilDate("2026-06-03".into())),
                "{\"day\":\"2026-06-03\"}",
            ),
            (
                TemporalRef::Range {
                    start: ts(0),
                    end: ts(100),
                },
                "{\"range\":{\"start\":0,\"end\":100}}",
            ),
            (
                TemporalRef::Approx {
                    center: ts(10),
                    fuzz_days: 2,
                },
                "{\"approx\":{\"center\":10,\"fuzz_days\":2}}",
            ),
            (
                TemporalRef::Recurring(Rrule("FREQ=WEEKLY".into())),
                "{\"recurring\":\"FREQ=WEEKLY\"}",
            ),
            (
                TemporalRef::BeforeAfter {
                    dir: Direction::After,
                    anchor: MemoryName::new("event/wedding"),
                },
                "{\"before_after\":{\"dir\":\"after\",\"anchor\":\"event/wedding\"}}",
            ),
        ];
        for (reference, json) in cases {
            assert_eq!(serde_json::to_string(&reference).unwrap(), json);
            assert_eq!(
                serde_json::from_str::<TemporalRef>(json).unwrap(),
                reference
            );
        }
    }
}
