//! Event-locating queries: finding conversations, sessions, memories, and entries within a run.

use std::collections::{BTreeMap, BTreeSet};

use zuihitsu::{
    ConversationId, EntryId, Event, EventPayload, Initiation, MemoryId, Seq, Teller, TemporalRef,
    TurnRole, Visibility,
};

use crate::analysis::EntryFacts;

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
/// the per-room slice of [`crate::analysis::agent_replies`], so a two-room scenario can probe each room's exposed
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

/// Each session's frozen brief and the working set it opened with, in open order — the
/// `SessionStarted` payloads' `brief` and `working_set`. Lets an oracle inspect what the composed
/// brief carried at a given open: whether a cold open re-surfaced an active thread (a non-empty
/// working set and a `# Active threads` section), or whether a warm carryover did.
pub fn session_briefs(events: &[Event]) -> Vec<(&str, &[MemoryId])> {
    events
        .iter()
        .filter_map(|event| match &event.payload {
            EventPayload::SessionStarted {
                brief, working_set, ..
            } => Some((brief.as_str(), working_set.as_slice())),
            _ => None,
        })
        .collect()
}

/// Whether each session, in open order, was seeded from a prior session's carried tail — its
/// `SessionStarted.seeded_from_turn` is set. A fresh first-contact open is `false`; a compaction,
/// idle, or recovery reopen that carried a raw-transcript tail is `true` (issue #86). Lets an oracle
/// assert the mechanism fired at a given seam without re-running the model.
pub fn session_seeds(events: &[Event]) -> Vec<bool> {
    events
        .iter()
        .filter_map(|event| match &event.payload {
            EventPayload::SessionStarted {
                seeded_from_turn, ..
            } => Some(seeded_from_turn.is_some()),
            _ => None,
        })
        .collect()
}

/// Whether a fired wake-up was raised into a session — the recurrence actually surfaced, not merely
/// got recorded.
pub fn scheduled_item_surfaced(events: &[Event]) -> bool {
    events
        .iter()
        .any(|event| matches!(&event.payload, EventPayload::ScheduledItemSurfaced { .. }))
}

/// The seq of the first participant `ConversationTurn` whose text is exactly `text`, if the run recorded
/// one — the anchor an oracle scopes an after-this-message query to, and the structural proof that a
/// specific inbound message landed durably as a participant turn.
pub fn participant_turn_seq(events: &[Event], text: &str) -> Option<Seq> {
    events.iter().find_map(|event| match &event.payload {
        EventPayload::ConversationTurn {
            role: TurnRole::Participant,
            text: turn_text,
            ..
        } if turn_text == text => Some(event.seq),
        _ => None,
    })
}

/// Whether a participant `ConversationTurn` whose text is exactly `text` was recorded — the platform
/// contract that every inbound message is durably logged as a participant turn before the agent answers,
/// regardless of whether the turn it opened went on to be superseded.
pub fn participant_turn_recorded(events: &[Event], text: &str) -> bool {
    participant_turn_seq(events, text).is_some()
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

/// One explicit attestation recorded in the run — a further teller standing behind an entry's fact
/// (spec §Visibility → attestations). The founding attestation is derived at materialization from the
/// entry's own append and is never logged, so this reads only the `EntryAttested` events an
/// auto-attest corroboration, a tier-1 cross-teller merge, or a tier-2 absorb-and-attest emitted:
/// which entry was endorsed, by which teller, and at what posture.
pub struct AttestationFacts {
    pub entry: EntryId,
    pub teller: Teller,
    pub posture: Visibility,
}

/// Every explicit `EntryAttested` the run recorded, in event order — the endorsements a corroboration
/// or a consolidation left on an entry, distinct from the founding attestation the fold derives.
pub fn attestations(events: &[Event]) -> Vec<AttestationFacts> {
    events
        .iter()
        .filter_map(|event| match &event.payload {
            EventPayload::EntryAttested {
                entry,
                teller,
                posture,
                ..
            } => Some(AttestationFacts {
                entry: *entry,
                teller: teller.clone(),
                posture: posture.clone(),
            }),
            _ => None,
        })
        .collect()
}

/// Whether `text` names `word` as a whole word (case-insensitive) — the identity-leak primitive for a
/// gate that must catch a handle stem rendered on a surface (`Frank`, `person/frank`, `Frank's`) without
/// false-positiving on a longer word that merely contains it (`frankly`, `graceful`). Tokenizes on any
/// non-alphanumeric boundary, so a stem embedded in a handle or a possessive still matches while a stem
/// buried inside an unrelated word does not.
pub fn mentions_word(text: &str, word: &str) -> bool {
    let word = word.to_lowercase();
    !word.is_empty()
        && text
            .to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .any(|token| token == word)
}

/// The set of entry ids that have been superseded, from `MemorySuperseded` events and
/// `EntriesConsolidated` source lists (both tombstone entries via the graph's `superseded_by`
/// column).
pub fn superseded_entry_ids(events: &[Event]) -> BTreeSet<EntryId> {
    let mut ids = BTreeSet::new();
    for event in events {
        match &event.payload {
            EventPayload::MemorySuperseded { entry, .. } => {
                ids.insert(*entry);
            }
            EventPayload::EntriesConsolidated { sources, .. } => {
                ids.extend(sources.iter().copied());
            }
            _ => {}
        }
    }
    ids
}

/// The set of entry ids withdrawn by a retraction (`EntryRetracted`) — hidden from every live surface
/// like superseded ones, but kept in history with a stated reason.
pub fn retracted_entry_ids(events: &[Event]) -> BTreeSet<EntryId> {
    events
        .iter()
        .filter_map(|event| match &event.payload {
            EventPayload::EntryRetracted { entry, .. } => Some(*entry),
            _ => None,
        })
        .collect()
}

/// Whether a live entry on a memory whose name contains `subject` has text that *exactly* matches
/// `needle` (case-insensitive). Unlike [`live_entry_on`], this does not match substrings — the
/// replacement entry from a consolidation may contain overlapping words, so an exact match is
/// needed to check whether a specific source entry is still live.
pub fn live_entry_exact(events: &[Event], subject: &str, needle: &str) -> bool {
    let hidden: BTreeSet<EntryId> = superseded_entry_ids(events)
        .union(&retracted_entry_ids(events))
        .copied()
        .collect();
    let subject = subject.to_lowercase();
    let needle = needle.trim().to_lowercase();
    entries(events).into_iter().any(|entry| {
        entry.memory.to_lowercase().contains(&subject)
            && entry.text.trim().to_lowercase() == needle
            && !hidden.contains(&entry.entry_id)
    })
}

/// Whether the run retracted an entry with a stated (non-empty) reason — the structured, auditable
/// withdrawal of a fact, distinct from an in-place supersession. The write path rejects an empty
/// reason, so a landed `EntryRetracted` always carries one; the trim guards against a whitespace-only
/// reason slipping the check in a future regression.
pub fn retraction_with_reason(events: &[Event]) -> bool {
    events.iter().any(|event| {
        matches!(
            &event.payload,
            EventPayload::EntryRetracted { reason, .. } if !reason.trim().is_empty()
        )
    })
}

/// Whether a *live* content entry whose text contains `needle` sits on a memory whose name contains
/// `subject` — an entry that was appended and neither superseded nor retracted. Matching is
/// case-insensitive and substring-based, so it reads the fact's presence however the run cased or
/// prefixed the handle and phrased the entry. Used to assert both that a mis-filed fact left no live
/// residue on the wrong memory and that it now lives on the right one.
pub fn live_entry_on(events: &[Event], subject: &str, needle: &str) -> bool {
    let hidden: BTreeSet<EntryId> = superseded_entry_ids(events)
        .union(&retracted_entry_ids(events))
        .copied()
        .collect();
    let subject = subject.to_lowercase();
    let needle = needle.to_lowercase();
    entries(events).into_iter().any(|entry| {
        entry.memory.to_lowercase().contains(&subject)
            && entry.text.to_lowercase().contains(&needle)
            && !hidden.contains(&entry.entry_id)
    })
}

/// How many `Attributed` entries the run recorded on a memory whose name contains `subject`. An
/// `Attributed` entry surfaces like a `Public` one but carries a "via <teller>" provenance marker and
/// is excluded from the memory's description — the posture a relayed secondhand fact is classified up
/// into. A count of two or more is the precondition the widened arbitration pool needs: two
/// relayed-but-conflicting accounts the agent marked `Attributed` are the case arbitration must still
/// catch even though neither is `Public`.
pub fn attributed_entries_on(events: &[Event], subject: &str) -> usize {
    let subject = subject.to_lowercase();
    entries(events)
        .into_iter()
        .filter(|entry| {
            entry.visibility == Visibility::Attributed
                && entry.memory.to_lowercase().contains(&subject)
        })
        .count()
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

/// Whether the run superseded a confidence entrusted by a participant: a `MemorySuperseded` whose
/// superseded entry was appended non-Public (`PrivateToTeller` or `Exclude`) by a conversation
/// participant. This is the forbidden mutation the foreign-confidence gate blocks — replacing what
/// someone else entrusted rather than appending a correction of one's own. It does not fire on the
/// agent superseding its own notes (teller `Agent`), nor on superseding a `Public` entry, so a
/// legitimate self-correction still reads as held.
pub fn superseded_participant_confidence(events: &[Event]) -> bool {
    let superseded = superseded_entry_ids(events);
    entries(events).into_iter().any(|entry| {
        superseded.contains(&entry.entry_id)
            && matches!(entry.told_by, Teller::Participant(_))
            && matches!(
                entry.visibility,
                Visibility::PrivateToTeller | Visibility::Exclude(_)
            )
    })
}

/// Whether a participant whose memory name contains `teller` appended content to a memory whose name
/// contains `subject` — the structural signal of a correction the agent recorded attributed to the
/// current speaker, rather than superseding the original teller's entry. Matching is
/// case-insensitive and substring-based so a `person/dave` teller on a `person/marcus` memory is found
/// however the run happened to case or prefix the handles.
pub fn participant_append_on(events: &[Event], subject: &str, teller: &str) -> bool {
    let names = memory_names(events);
    let subject = subject.to_lowercase();
    let teller = teller.to_lowercase();
    entries(events).into_iter().any(|entry| {
        entry.memory.to_lowercase().contains(&subject)
            && matches!(
                entry.told_by,
                Teller::Participant(id)
                    if names.get(&id).is_some_and(|name| name.to_lowercase().contains(&teller))
            )
    })
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
