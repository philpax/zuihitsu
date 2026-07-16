//! Temporal and occurrence state: entry occurrences, temporal-reference resolution, and lexical
//! leak backstops.

use std::collections::BTreeMap;

use zuihitsu::{
    BEFORE_AFTER_EPSILON_MILLIS, EntryId, Event, EventPayload, MemoryName, TemporalRef, Timestamp,
};

use crate::analysis::events::memory_names;

/// One content entry's full temporal picture: its `MemoryContentAppended` (the text, the memory it
/// landed on, and when it was asserted, plus any occurrence stamped *at append* — the authored slot)
/// joined with any later `EntryTemporalResolved` (the occurrence the turn-end extraction pass resolved
/// — the extracted slot). The two slots let an oracle tell an authored date from an extracted one on the
/// same entry: the distinction the search-hit and neighborhood-line projections lean on when they prefer
/// authored over extracted, and the one a temporal-honesty oracle checks for a fabricated resolution.
pub struct EntryOccurrence {
    pub memory: String,
    pub text: String,
    pub asserted_at: Timestamp,
    /// The occurrence the agent stamped at append time; `None` when it wrote the entry untimed.
    pub authored: Option<TemporalRef>,
    /// The occurrence the turn-end extraction pass resolved later; `None` when it left the entry
    /// unextracted (or could not parse the model's string).
    pub extracted: Option<TemporalRef>,
}

/// Every content entry's temporal picture, in append order — each `MemoryContentAppended` joined with
/// any later `EntryTemporalResolved` on the same entry (see [`EntryOccurrence`]).
pub fn entry_occurrences(events: &[Event]) -> Vec<EntryOccurrence> {
    let names = memory_names(events);
    let mut occurrences: Vec<EntryOccurrence> = Vec::new();
    let mut index: BTreeMap<EntryId, usize> = BTreeMap::new();
    for event in events {
        match &event.payload {
            EventPayload::MemoryContentAppended {
                id,
                entry_id,
                asserted_at,
                occurred_at,
                text,
                ..
            } => {
                index.insert(*entry_id, occurrences.len());
                occurrences.push(EntryOccurrence {
                    memory: names.get(id).cloned().unwrap_or_default(),
                    text: text.clone(),
                    asserted_at: *asserted_at,
                    authored: occurred_at.clone(),
                    extracted: None,
                });
            }
            EventPayload::EntryTemporalResolved {
                entry_id,
                occurred_at,
                ..
            } => {
                if let Some(&position) = index.get(entry_id) {
                    occurrences[position].extracted = Some(occurred_at.clone());
                }
            }
            _ => {}
        }
    }
    occurrences
}

/// Whether a temporal reference pins a fixed instant — an `Instant`, `Day`, `Range`, or `Approx`, all of
/// which denormalize to a representative sort instant. A `BeforeAfter` (relative to another memory) and a
/// `Recurring` rule have no fixed instant of their own, so they read as *not* concrete — which is the
/// honest-anchoring outcome for a phrase that names another event rather than the speaker's now. The
/// anchor is resolved with `None`, so a `BeforeAfter` stays instant-less here rather than borrowing one.
pub fn resolves_to_instant(occurred_at: &TemporalRef) -> bool {
    occurred_at
        .bounds(None, BEFORE_AFTER_EPSILON_MILLIS)
        .sort
        .is_some()
}

/// Whether a temporal reference resolves to a concrete instant within `window_ms` of `anchor_ms` — the
/// structural signal of a clock-anchored resolution when `anchor_ms` is the conversation's own now. A
/// `BeforeAfter` or `Recurring` reference has no fixed instant (see [`resolves_to_instant`]), so it is
/// never "near" anything: exactly the honest outcome an oracle wants to let pass.
pub fn resolves_near(occurred_at: &TemporalRef, anchor_ms: i64, window_ms: i64) -> bool {
    occurred_at
        .bounds(None, BEFORE_AFTER_EPSILON_MILLIS)
        .sort
        .is_some_and(|sort| (sort.as_millis() - anchor_ms).abs() <= window_ms)
}

/// The anchor memory a `BeforeAfter` reference names, if `occurred_at` is one — the honest resolution for
/// a phrase anchored to another event rather than to the speaker's now (spec §Time → the anchor rule).
pub fn before_after_anchor(occurred_at: &TemporalRef) -> Option<&MemoryName> {
    match occurred_at {
        TemporalRef::BeforeAfter { anchor, .. } => Some(anchor),
        _ => None,
    }
}

/// A crude lexical leak backstop: the subject term co-occurring with any of `terms` in the text. A dumb
/// floor under the judge — an obvious leak can't slip a judge hiccup — never the primary signal.
pub fn lexical_leak(text: &str, subject: &str, terms: &[&str]) -> bool {
    let lower = text.to_lowercase();
    lower.contains(&subject.to_lowercase()) && terms.iter().any(|term| lower.contains(term))
}

/// Whether `script` calls `path`: an occurrence of `path` immediately followed (whitespace aside) by an
/// opening parenthesis. Not a full parse — it does not exclude occurrences inside strings or comments —
/// but it distinguishes a call from an incidental mention, which is what the oracles need.
pub(crate) fn script_calls(script: &str, path: &str) -> bool {
    let mut from = 0;
    while let Some(found) = script[from..].find(path) {
        let after = from + found + path.len();
        if script[after..].trim_start().starts_with('(') {
            return true;
        }
        from = after;
    }
    false
}
