//! Reading a run's event log for assessment — the deterministic side of an oracle (spec §Validation →
//! assessment is a pure function of the log). Everything here is a query over `&[Event]`, so it works
//! identically on a live run and on a stored package being re-assessed.

use std::collections::BTreeMap;

use zuihitsu::{Event, EventPayload, MemoryId, TemporalRef, TurnRole};

/// Every agent reply, in order.
pub fn agent_replies(events: &[Event]) -> Vec<&str> {
    events
        .iter()
        .filter_map(|event| match &event.payload {
            EventPayload::ConversationTurn {
                role: TurnRole::Agent,
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
