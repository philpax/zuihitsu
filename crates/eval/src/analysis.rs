//! Reading a run's event log for assessment — the deterministic side of an oracle (spec §Validation →
//! assessment is a pure function of the log). Everything here is a query over `&[Event]`, so it works
//! identically on a live run and on a stored package being re-assessed.

use std::collections::{BTreeMap, BTreeSet};

use zuihitsu::{
    Event, EventPayload, Initiation, MemoryId, Teller, TemporalRef, TurnRole, Visibility,
    Volatility,
};

/// One durable content entry, projected from a `MemoryContentAppended` for assessment: which memory it
/// landed on, its text, the visibility it was written with, and who it is attributed to (so an oracle
/// can catch a relayed fact re-recorded under the wrong teller).
pub struct EntryFacts {
    pub memory: String,
    pub text: String,
    pub visibility: Visibility,
    pub told_by: Teller,
}

/// Every agent reply to a participant, in order. Only `Responding` turns count: an `Initiated` agent
/// turn is the agent acting unprompted — the pre-compaction flush, a wake-up — self-directed bookkeeping
/// addressed to no one, not a reply. Under a tight compaction budget every turn trails a flush, so
/// including those would let a flush summary stand in for the agent's actual answer to a probe.
pub fn agent_replies(events: &[Event]) -> Vec<&str> {
    events
        .iter()
        .filter_map(|event| match &event.payload {
            EventPayload::ConversationTurn {
                role: TurnRole::Agent,
                initiation: Initiation::Responding,
                text,
                ..
            } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

/// The agent's last reply, if it spoke.
pub fn last_agent_reply(events: &[Event]) -> Option<&str> {
    agent_replies(events).into_iter().last()
}

/// Every Lua block the agent executed, in order.
pub fn lua_scripts(events: &[Event]) -> Vec<&str> {
    events
        .iter()
        .filter_map(|event| match &event.payload {
            EventPayload::LuaExecuted { script, .. } => Some(script.as_str()),
            _ => None,
        })
        .collect()
}

/// Whether any executed Lua block *calls* `path` as a function — the name immediately followed by `(`
/// (whitespace allowed). Stricter than a bare substring search: a mention of the name in a comment, a
/// string, or a longer identifier does not count, so the oracle asserts the agent reached for the call
/// rather than merely that the text appeared.
pub fn lua_called(events: &[Event], path: &str) -> bool {
    lua_scripts(events)
        .iter()
        .any(|script| script_calls(script, path))
}

/// Whether the tag named `tag` was applied to any memory.
pub fn tag_applied(events: &[Event], tag: &str) -> bool {
    events.iter().any(|event| {
        matches!(&event.payload, EventPayload::TagAppliedToMemory { tag: applied, .. } if applied.as_str() == tag)
    })
}

/// Whether a link of the relation named `relation` was created — the structural signal that the agent
/// recorded a typed edge, not just any link or prose.
pub fn link_created_with(events: &[Event], relation: &str) -> bool {
    events.iter().any(|event| {
        matches!(&event.payload, EventPayload::LinkCreated { relation: created, .. } if created.as_str() == relation)
    })
}

/// Whether the agent proposed a cross-platform merge (a `MergeProposed`) — the agent's judgment that
/// two stubs may be one person, before any adjudication.
pub fn merge_proposed(events: &[Event]) -> bool {
    events
        .iter()
        .any(|event| matches!(&event.payload, EventPayload::MergeProposed { .. }))
}

/// Whether the run actually merged two stubs: an adjudication accepted *and* authored the `same_as`
/// (`source = Adjudicated`). This is the surfacing-changing outcome — distinct from merely proposing,
/// which is inert.
pub fn merge_committed(events: &[Event]) -> bool {
    events.iter().any(|event| {
        matches!(
            &event.payload,
            EventPayload::LinkCreated { relation, source, .. }
                if relation.as_str() == "same_as" && source.as_str() == "Adjudicated"
        )
    })
}

/// Whether the run marked any memory `High` volatility — the agent classified a fast-changing memory
/// so its facts decay and can read as stale (spec §Recency and volatility).
pub fn volatility_set_high(events: &[Event]) -> bool {
    events.iter().any(|event| {
        matches!(
            &event.payload,
            EventPayload::MemoryVolatilitySet {
                volatility: Volatility::High,
                ..
            }
        )
    })
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
                text,
                visibility,
                told_by,
                ..
            } => Some(EntryFacts {
                memory: names.get(id).cloned().unwrap_or_default(),
                text: text.clone(),
                visibility: visibility.clone(),
                told_by: told_by.clone(),
            }),
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

/// A crude lexical leak backstop: the subject term co-occurring with any of `terms` in the text. A dumb
/// floor under the judge — an obvious leak can't slip a judge hiccup — never the primary signal.
pub fn lexical_leak(text: &str, subject: &str, terms: &[&str]) -> bool {
    let lower = text.to_lowercase();
    lower.contains(&subject.to_lowercase()) && terms.iter().any(|term| lower.contains(term))
}

/// Whether `script` calls `path`: an occurrence of `path` immediately followed (whitespace aside) by an
/// opening parenthesis. Not a full parse — it does not exclude occurrences inside strings or comments —
/// but it distinguishes a call from an incidental mention, which is what the oracles need.
fn script_calls(script: &str, path: &str) -> bool {
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
