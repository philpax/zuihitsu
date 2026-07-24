//! Resolve extracted occurrences to `EntryTemporalResolved` events, shared by the public synthesis
//! pass and the focused private-entry extraction pass.

use std::collections::{BTreeMap, BTreeSet};

use crate::{
    agent::turn::describe::ExtractedOccurrence,
    event::{EventPayload, ProducedBy},
    graph::{EntryView, MemoryView},
    ids::{EntryId, MemoryId},
    time::{MILLIS_PER_DAY, TemporalRef, Timestamp},
};

/// The per-memory read context a resolution pass reasons over: the entries shown to the model
/// (1-based statement numbers key into `list`), the new untimed entries it may resolve, the memory
/// itself, the pass's current time, and the live occurrences already sitting on the memory's entries.
/// Bundled so the current-day guard's inputs travel together rather than as a fistful of positional
/// arguments.
pub(super) struct ResolveContext<'a> {
    pub(super) list: &'a [EntryView],
    pub(super) eligible: &'a BTreeMap<EntryId, MemoryId>,
    pub(super) memory: &'a MemoryView,
    pub(super) now: Timestamp,
    /// The occurrences already carried by the memory's live entries (the description mirror's authored
    /// date among them). A `Day`- or `Instant`-shaped sibling on a day other than `now`'s turns the
    /// current-day guard on: it marks a freshly extracted current-day resolution as a back-pointing phrase
    /// mis-anchored to "Current time" rather than a genuine same-day fact.
    pub(super) siblings: &'a [TemporalRef],
}

/// Resolve the extracted `occurrences` for the entries in `ctx.list` (1-based statement numbers),
/// pushing an `EntryTemporalResolved` for each new, untimed entry, once. Shared by the public synthesis
/// pass and the focused private-entry extraction pass, so each only resolves the entries it was shown.
///
/// A resolution that lands on `ctx.now`'s own day is suppressed when a sibling entry of the memory
/// carries an occurrence on a different day (see [`lands_on_now`] and [`ResolveContext::siblings`]): an
/// extracted current-day date beside a differently-dated sibling reads as a back-pointing phrase ("this date")
/// mis-anchored to the conversation's "Current time", so the entry stays untimed rather than
/// contradicting the authored occurrence. The suppression is recorded as an `EntryTemporalResolveFailed`.
pub(super) fn resolve_occurrences(
    occurrences: Vec<ExtractedOccurrence>,
    ctx: &ResolveContext<'_>,
    resolved: &mut BTreeSet<EntryId>,
    provenance: &ProducedBy,
    events: &mut Vec<EventPayload>,
) {
    for occurrence in occurrences {
        // The statement number is 1-based into the entries listed in the prompt.
        let Some(entry) = occurrence
            .entry
            .checked_sub(1)
            .and_then(|i| ctx.list.get(i))
        else {
            continue;
        };
        // Only a new, untimed entry; skip anything else the model keyed (an entry already timed,
        // explicitly set, or a class sibling not written this turn), and resolve each once.
        let Some(&entry_memory) = ctx.eligible.get(&entry.entry_id) else {
            continue;
        };
        if !resolved.insert(entry.entry_id) {
            continue;
        }
        let raw_occurred_at = occurrence.occurred_at.clone();
        let occurred_at = match occurrence.occurred_at.into_temporal_ref() {
            Some(occurred_at) => occurred_at,
            None => {
                let raw = serde_json::to_string(&raw_occurred_at).unwrap_or_default();
                tracing::warn!(
                    memory = %ctx.memory.name.as_str(),
                    %raw,
                    "dropping an unparseable extracted occurrence; the model emitted a temporal reference this build cannot interpret"
                );
                events.push(EventPayload::entry_temporal_resolve_failed(
                    entry_memory,
                    entry.entry_id,
                    raw,
                    "unparseable temporal reference".to_owned(),
                    Some(provenance.clone()),
                ));
                continue;
            }
        };
        // An extracted occurrence on the current day beside a differently-dated sibling reads as a
        // phrase mis-anchored to "Current time"; drop it so the entry stays untimed rather than
        // contradicting the authored date, and record the suppression for review.
        if lands_on_now(&occurred_at, ctx.now) && has_differently_dated_sibling(ctx) {
            let raw = serde_json::to_string(&raw_occurred_at).unwrap_or_default();
            tracing::debug!(
                memory = %ctx.memory.name.as_str(),
                %raw,
                "suppressing a current-day resolution beside a differently-dated sibling; leaving the entry untimed"
            );
            events.push(EventPayload::entry_temporal_resolve_failed(
                entry_memory,
                entry.entry_id,
                raw,
                "an extracted occurrence on the current day beside a differently-dated sibling reads \
                 as a back-pointing phrase mis-anchored to \"Current time\"; the entry stays untimed"
                    .to_owned(),
                Some(provenance.clone()),
            ));
            continue;
        }
        events.push(EventPayload::entry_temporal_resolved(
            entry_memory,
            entry.entry_id,
            Some(occurred_at),
            Some(provenance.clone()),
        ));
    }
}

/// Whether a resolution `occurred_at` denotes exactly `now`'s own civil day. Conservative on purpose:
/// only the two shapes that name a single day — an `Instant` (the day it falls in) and a `Day` — can
/// match, so a `Range`, `Approx`, `Recurring`, or `BeforeAfter` (each spanning or deferring more than
/// one day) never trips the guard and applies as extracted.
fn lands_on_now(occurred_at: &TemporalRef, now: Timestamp) -> bool {
    single_day_midnight(occurred_at) == Some(day_midnight(now.as_millisecond()))
}

/// Whether some sibling occurrence names a single civil day other than `now`'s — the second half of the
/// guard's condition. Only single-day siblings count, matching [`lands_on_now`]'s conservatism, so a
/// vague sibling never forces a suppression.
fn has_differently_dated_sibling(ctx: &ResolveContext<'_>) -> bool {
    let today = day_midnight(ctx.now.as_millisecond());
    ctx.siblings
        .iter()
        .filter_map(single_day_midnight)
        .any(|midnight| midnight != today)
}

/// The midnight-UTC millisecond of a temporal ref that denotes exactly one civil day — an `Instant`
/// (the day it falls in) or a `Day` — or `None` for the vaguer shapes.
fn single_day_midnight(occurred_at: &TemporalRef) -> Option<i64> {
    match occurred_at {
        TemporalRef::Instant(at) => Some(day_midnight(at.as_millisecond())),
        TemporalRef::Day(date) => date.midnight_millis(),
        TemporalRef::Range { .. }
        | TemporalRef::Approx { .. }
        | TemporalRef::Recurring(_)
        | TemporalRef::BeforeAfter { .. } => None,
    }
}

/// Midnight UTC of the civil day a millisecond timestamp falls in. `rem_euclid` floors toward the
/// earlier day for a pre-epoch instant, so the whole day maps to its own midnight.
fn day_midnight(millis: i64) -> i64 {
    millis - millis.rem_euclid(MILLIS_PER_DAY)
}
