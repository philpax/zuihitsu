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
///
/// The guard reads the entry's own text as well, but that arrives per-entry from `list` rather than
/// here, since it varies across the statements one pass resolves.
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
/// A resolution that lands on `ctx.now`'s own day is suppressed in two cases (see
/// [`current_day_suppression`]): when a sibling entry carries an occurrence on a different day, so the
/// date reads as a back-pointing phrase ("this date") mis-anchored to the conversation's "Current
/// time"; and when the statement names no time of its own, so the current day can only have come from
/// the clock rather than from what was said. Either way the entry stays untimed rather than carrying a
/// date it never stated, and the suppression is recorded as an `EntryTemporalResolveFailed`.
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
        // A current-day resolution the statement itself does not support reads as the conversation's
        // "Current time" leaking into the extraction; drop it so the entry stays untimed rather than
        // carrying a date it never stated, and record the suppression for review.
        if let Some(reason) = current_day_suppression(&occurred_at, entry, ctx) {
            let raw = serde_json::to_string(&raw_occurred_at).unwrap_or_default();
            tracing::debug!(
                memory = %ctx.memory.name.as_str(),
                %raw,
                %reason,
                "suppressing a current-day resolution; leaving the entry untimed"
            );
            events.push(EventPayload::entry_temporal_resolve_failed(
                entry_memory,
                entry.entry_id,
                raw,
                reason.to_string(),
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

/// Why a resolution onto the current day is not credible for this entry, or `None` when it stands.
/// Both branches answer the same question — did this date come from the statement, or from the
/// conversation's "Current time"? — and both fail safe to leaving the entry untimed, which the
/// extraction prompt itself names as the better direction: a fabricated now-relative date reads back
/// as fact, where no date merely sends the reader to the entry.
fn current_day_suppression(
    occurred_at: &TemporalRef,
    entry: &EntryView,
    ctx: &ResolveContext<'_>,
) -> Option<CurrentDaySuppression> {
    if !lands_on_now(occurred_at, ctx.now) {
        return None;
    }
    if has_differently_dated_sibling(ctx) {
        return Some(CurrentDaySuppression::MisanchoredBesideSibling);
    }
    if !states_a_time(&entry.text, ctx.now) {
        return Some(CurrentDaySuppression::StatementNamesNoTime);
    }
    None
}

/// Why a current-day resolution was dropped. The two reasons are a closed set, so they ride as a type
/// rather than as prose assembled at the call site, and a test can name the branch it expects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CurrentDaySuppression {
    /// A sibling entry carries a different single day, so the resolution reads as a back-pointing
    /// phrase ("this date") mis-anchored to the conversation's "Current time".
    MisanchoredBesideSibling,
    /// The statement names no time of its own, so the current day can only have come from the clock.
    StatementNamesNoTime,
}

impl std::fmt::Display for CurrentDaySuppression {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let reason = match self {
            Self::MisanchoredBesideSibling => {
                "an extracted occurrence on the current day beside a differently-dated sibling reads \
                 as a back-pointing phrase mis-anchored to \"Current time\"; the entry stays untimed"
            }
            Self::StatementNamesNoTime => {
                "an extracted occurrence on the current day for a statement that names no time of \
                 its own reads as the conversation's \"Current time\" standing in for a date the \
                 statement never gave; the entry stays untimed"
            }
        };
        formatter.write_str(reason)
    }
}

/// Whether an entry's own text names a time that could have pinned `now`'s day — a date-shaped run of
/// characters, a same-day deictic, or the name of `now`'s own weekday. A statement with none of these
/// cannot have supplied the current day, so a resolution landing there came from the clock.
///
/// The cues are deliberately narrow, because the test that matters is not "does this text mention a
/// number" but "could this text have produced *today's* date". So:
///
/// - A digit counts only in a date shape. A bare digit is far commoner as a floor, a quarter, a
///   version, or a street number than as a date, and admitting it readmits the fabrication the guard
///   exists to catch.
/// - A weekday counts only when it is `now`'s weekday. "Moved the standup to Thursday", said on a
///   Monday, cannot be why a resolution landed on Monday.
/// - Vague now-words ("recently", "lately", "soon") are absent: they anchor to the moment of speaking
///   without pinning a day, which is exactly the case the extraction prompt says to omit.
/// - A bare month name is absent: without a day beside it, it names a month rather than a day.
///
/// The cue lists are English, matching the prompt surface the extraction runs against. For text in
/// another language only the date shapes match, so the guard suppresses more readily — the safe
/// direction, since a suppressed entry is merely untimed.
pub(super) fn states_a_time(text: &str, now: Timestamp) -> bool {
    /// Phrases that place a statement on the speaker's own day. Each must be a distinct substring:
    /// "earlier today" and the like are covered by "today" already and would never change an answer.
    const SAME_DAY_CUES: [&str; 7] = [
        "today",
        "tonight",
        "this morning",
        "this afternoon",
        "this evening",
        "right now",
        "just now",
    ];

    if has_date_shape(text) {
        return true;
    }
    let lowered = text.to_lowercase();
    if SAME_DAY_CUES.iter().any(|cue| lowered.contains(cue)) {
        return true;
    }
    crate::time::today_weekday(now).is_some_and(|weekday| lowered.contains(&weekday))
}

/// Whether `text` contains a run that reads as a written date or clock time: a digit adjacent to
/// another date-bearing token — a second digit, a separator, an ordinal suffix, or a month name. A
/// lone digit surrounded by ordinary words is not a date, which is the whole point of the test.
fn has_date_shape(text: &str) -> bool {
    /// Month names, which turn an adjacent bare number into a day-of-month.
    const MONTHS: [&str; 12] = [
        "january",
        "february",
        "march",
        "april",
        "may",
        "june",
        "july",
        "august",
        "september",
        "october",
        "november",
        "december",
    ];

    /// The suffixes that turn a preceding number into a day-of-month or a clock time.
    const SUFFIXES: [&str; 6] = ["st", "nd", "rd", "th", "am", "pm"];

    let lowered = text.to_lowercase();
    let tokens: Vec<String> = lowered
        .split_whitespace()
        .map(|token| {
            token
                .trim_matches(|character: char| !character.is_alphanumeric())
                .to_owned()
        })
        .collect();

    // Digits on both sides of a separator: "2026-06-08", "6/8", "9:30".
    let separated = |token: &str, separator: char| {
        token
            .split(separator)
            .filter(|part| !part.is_empty() && part.chars().all(char::is_numeric))
            .count()
            >= 2
    };
    // A number carrying an ordinal or meridiem suffix: "8th", "7pm".
    let suffixed = |token: &str| {
        SUFFIXES.iter().any(|suffix| {
            token
                .strip_suffix(suffix)
                .is_some_and(|head| head.chars().next_back().is_some_and(char::is_numeric))
        })
    };
    let is_month = |token: &String| MONTHS.contains(&token.as_str());

    tokens.iter().enumerate().any(|(index, token)| {
        if !token.chars().any(char::is_numeric) {
            return false;
        }
        if separated(token, '-')
            || separated(token, '/')
            || separated(token, ':')
            || suffixed(token)
        {
            return true;
        }
        // A bare number names a day only beside a month ("8 June", "June 8"). Alone it is far likelier
        // a floor, a quarter, a version, a count, or a bare year — none of which pin a day.
        index
            .checked_sub(1)
            .is_some_and(|before| tokens.get(before).is_some_and(is_month))
            || tokens.get(index + 1).is_some_and(is_month)
    })
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
