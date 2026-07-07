//! Resolve extracted occurrences to `EntryTemporalResolved` events, shared by the public synthesis
//! pass and the focused private-entry extraction pass.

use std::collections::{BTreeMap, BTreeSet};

use crate::{
    event::{EventPayload, ProducedBy},
    graph::{EntryView, MemoryView},
    ids::{EntryId, MemoryId},
};

use super::ExtractedOccurrence;

/// Resolve the extracted `occurrences` for the entries `list` (1-based statement numbers), pushing an
/// `EntryTemporalResolved` for each new, untimed entry, once. Shared by the public synthesis pass and
/// the focused private-entry extraction pass, so each only resolves the entries it was shown.
pub(super) fn resolve_occurrences(
    occurrences: Vec<ExtractedOccurrence>,
    list: &[EntryView],
    eligible: &BTreeMap<EntryId, MemoryId>,
    resolved: &mut BTreeSet<EntryId>,
    provenance: &ProducedBy,
    memory: &MemoryView,
    events: &mut Vec<EventPayload>,
) {
    for occurrence in occurrences {
        // The statement number is 1-based into the entries listed in the prompt.
        let Some(entry) = occurrence.entry.checked_sub(1).and_then(|i| list.get(i)) else {
            continue;
        };
        // Only a new, untimed entry; skip anything else the model keyed (an entry already timed,
        // explicitly set, or a class sibling not written this turn), and resolve each once.
        let Some(&entry_memory) = eligible.get(&entry.entry_id) else {
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
                    memory = %memory.name.as_str(),
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
        events.push(EventPayload::entry_temporal_resolved(
            entry_memory,
            entry.entry_id,
            occurred_at,
            Some(provenance.clone()),
        ));
    }
}
