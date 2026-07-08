//! Event-locating queries: finding conversations, sessions, memories, and entries within a run.

use std::collections::{BTreeMap, BTreeSet};

use zuihitsu::{
    ConversationId, EntryId, Event, EventPayload, Initiation, MemoryId, Teller, TemporalRef,
    TurnRole, Visibility,
};

use super::EntryFacts;

/// The durable conversation opened for `platform`/`scope`, if the run reached that room — so a
/// multi-room scenario can scope a query to one room's events.
pub fn conversation_id(events: &[Event], platform: &str, scope: &str) -> Option<ConversationId> {
    events.iter().find_map(|event| match &event.payload {
        EventPayload::ConversationStarted { id, locator, .. }
            if locator.platform.as_str() == platform && locator.scope_path.as_str() == scope =>
        {
            Some(*id)
        }
        _ => None,
    })
}

/// Every agent reply to a participant within one room (located by `platform`/`scope`), in order —
/// the per-room slice of [`super::agent_replies`], so a two-room scenario can probe each room's exposed
/// surface separately.
pub fn agent_replies_in<'a>(events: &'a [Event], platform: &str, scope: &str) -> Vec<&'a str> {
    let Some(conversation) = conversation_id(events, platform, scope) else {
        return Vec::new();
    };
    events
        .iter()
        .filter_map(|event| match &event.payload {
            EventPayload::ConversationTurn {
                conversation: turn_conversation,
                role: TurnRole::Agent,
                initiation: Initiation::Responding,
                text,
                ..
            } if *turn_conversation == conversation => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

/// How many sessions opened in the run — one more than the number of cuts (compaction or idle
/// re-segmentation), so `session_count - 1` counts the seams the carryover crossed.
pub fn session_count(events: &[Event]) -> usize {
    events
        .iter()
        .filter(|event| matches!(&event.payload, EventPayload::SessionStarted { .. }))
        .count()
}

/// Whether a fired wake-up was raised into a session — the recurrence actually surfaced, not merely
/// got recorded.
pub fn scheduled_item_surfaced(events: &[Event]) -> bool {
    events
        .iter()
        .any(|event| matches!(&event.payload, EventPayload::ScheduledItemSurfaced { .. }))
}

/// The id of the memory created under the exact handle `name`, if the run created one — so an oracle
/// that needs a specific memory's endpoints can resolve it from the log rather than threading run-time
/// ids into the assessment.
pub fn memory_id_named(events: &[Event], name: &str) -> Option<MemoryId> {
    events.iter().find_map(|event| match &event.payload {
        EventPayload::MemoryCreated { id, name: created } if created.as_str() == name => Some(*id),
        _ => None,
    })
}

/// The id → name map of every memory created in the run.
pub fn memory_names(events: &[Event]) -> BTreeMap<MemoryId, String> {
    events
        .iter()
        .filter_map(|event| match &event.payload {
            EventPayload::MemoryCreated { id, name } => Some((*id, name.as_str().to_owned())),
            _ => None,
        })
        .collect()
}

/// The names of memories that received a recurring occurrence.
pub fn recurring_memory_names(events: &[Event]) -> Vec<String> {
    let names = memory_names(events);
    events
        .iter()
        .filter_map(|event| match &event.payload {
            EventPayload::MemoryContentAppended {
                id,
                occurred_at: Some(TemporalRef::Recurring(_)),
                ..
            } => names.get(id).cloned(),
            _ => None,
        })
        .collect()
}

/// Every durable content entry written in the run, with the memory it landed on and its visibility.
pub fn entries(events: &[Event]) -> Vec<EntryFacts> {
    let names = memory_names(events);
    events
        .iter()
        .filter_map(|event| match &event.payload {
            EventPayload::MemoryContentAppended {
                id,
                entry_id,
                text,
                visibility,
                told_by,
                ..
            } => Some(EntryFacts {
                memory: names.get(id).cloned().unwrap_or_default(),
                text: text.clone(),
                visibility: visibility.clone(),
                told_by: told_by.clone(),
                entry_id: *entry_id,
            }),
            _ => None,
        })
        .collect()
}

/// The set of entry ids that have been superseded, from `MemorySuperseded` events.
pub fn superseded_entry_ids(events: &[Event]) -> BTreeSet<EntryId> {
    events
        .iter()
        .filter_map(|event| match &event.payload {
            EventPayload::MemorySuperseded { entry, .. } => Some(*entry),
            _ => None,
        })
        .collect()
}

/// How many distinct conversation participants are credited as the teller of a non-Public entry on a
/// memory whose name contains `subject`. A confidence about `subject` should be held under exactly its
/// one original teller; a count above one means it was re-recorded under a second teller — typically
/// the current speaker when the agent redundantly re-wrote a fact it already held — which silently
/// re-keys whose private note it is and can later surface the confidence to the wrong person.
pub fn private_tellers_of(events: &[Event], subject: &str) -> usize {
    let subject = subject.to_lowercase();
    let mut tellers = BTreeSet::new();
    for entry in entries(events) {
        if entry.visibility != Visibility::Public
            && entry.memory.to_lowercase().contains(&subject)
            && let Teller::Participant(id) = entry.told_by
        {
            tellers.insert(id);
        }
    }
    tellers.len()
}

/// Each memory's latest synthesized description (the always-visible summary), as `(name, text)`. A
/// later regeneration supersedes an earlier one, so only the last per memory is kept.
pub fn descriptions(events: &[Event]) -> Vec<(String, String)> {
    let names = memory_names(events);
    let mut latest: BTreeMap<MemoryId, String> = BTreeMap::new();
    for event in events {
        if let EventPayload::MemoryDescriptionRegenerated { id, new_text, .. } = &event.payload {
            latest.insert(*id, new_text.clone());
        }
    }
    latest
        .into_iter()
        .map(|(id, text)| (names.get(&id).cloned().unwrap_or_default(), text))
        .collect()
}

/// The names of every memory minted in the run whose name begins with `prefix` (e.g. `event/`), for
/// asserting the agent reused an existing memory rather than creating a duplicate of the same entity.
pub fn memories_in_namespace(events: &[Event], prefix: &str) -> Vec<String> {
    memory_names(events)
        .into_values()
        .filter(|name| name.starts_with(prefix))
        .collect()
}

/// Whether the run superseded any entry — the structured "this supersedes that" move that records an
/// explicit correction or update in state, rather than leaving the stale value standing or only saying
/// so in a reply. The signal that a correction landed durably (distinct from a genuine contradiction,
/// where both accounts are kept and the synthesis arbitrates instead).
pub fn any_superseded(events: &[Event]) -> bool {
    events
        .iter()
        .any(|event| matches!(event.payload, EventPayload::MemorySuperseded { .. }))
}

/// The reconciling statements of every belief arbitration the run recorded.
pub fn arbitrations(events: &[Event]) -> Vec<String> {
    events
        .iter()
        .filter_map(|event| match &event.payload {
            EventPayload::BeliefArbitrated { resolution, .. } => Some(resolution.statement.clone()),
            _ => None,
        })
        .collect()
}

/// Whether the run recorded a *both-stand* arbitration: one that credits neither competing entry
/// (`credited` empty), keeping both accounts standing rather than resolving the contradiction to one
/// side. This is the faithful signal for a genuine unresolved conflict — distinct from an arbitration
/// that credits a side, which is supersession by another name (one account chosen over the other).
pub fn both_stand_arbitration(events: &[Event]) -> bool {
    events.iter().any(|event| {
        matches!(
            &event.payload,
            EventPayload::BeliefArbitrated { resolution, .. } if resolution.credited.is_empty()
        )
    })
}

/// Whether the run recorded any recurring occurrence — emitted inline on a content append or resolved
/// by the turn-end temporal extraction. Either path proves the model produced a `Recurring` reference
/// rather than flattening "every Tuesday" to a single day.
pub fn has_recurring_occurrence(events: &[Event]) -> bool {
    events.iter().any(|event| {
        matches!(
            &event.payload,
            EventPayload::MemoryContentAppended {
                occurred_at: Some(TemporalRef::Recurring(_)),
                ..
            } | EventPayload::EntryTemporalResolved {
                occurred_at: TemporalRef::Recurring(_),
                ..
            }
        )
    })
}
