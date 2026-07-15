//! Turn ID mapping: Discord message IDs to zuihitsu `TurnId`s for `[turn:<id>]` token injection.
//!
//! Maps both agent responses and participant messages to their zuihitsu turn IDs. When a Discord
//! user replies to a mapped message, the connector injects a `[turn:<id>]` token into the message
//! text before forwarding to the platform API.

use std::collections::HashMap;

use serenity::model::id::MessageId;
use zuihitsu_core::{ids::TurnId, turn_ref};

/// An in-memory mapping from Discord message IDs to zuihitsu turn IDs. Not persisted — a restart
/// loses it, which is fine (the agent still has the full conversation history in its event log).
/// Bounded by random eviction when at capacity (keep last N=1000 entries).
pub struct TurnMap {
    map: HashMap<MessageId, TurnId>,
    capacity: usize,
}

impl TurnMap {
    pub fn new() -> Self {
        Self::with_capacity(1000)
    }

    pub fn with_capacity(capacity: usize) -> Self {
        TurnMap {
            map: HashMap::new(),
            capacity,
        }
    }

    /// Record a mapping from a Discord message ID to a zuihitsu turn ID.
    pub fn record(&mut self, message_id: MessageId, turn_id: TurnId) {
        // Evict the oldest entry if at capacity. HashMap has no insertion order, so this is a
        // random eviction — acceptable for an ephemeral cache.
        if self.map.len() >= self.capacity
            && let Some(&key) = self.map.keys().next()
        {
            self.map.remove(&key);
        }
        self.map.insert(message_id, turn_id);
    }

    /// Look up the turn ID for a Discord message, if mapped.
    pub fn get(&self, message_id: &MessageId) -> Option<TurnId> {
        self.map.get(message_id).copied()
    }

    /// If `referenced_message_id` is mapped, inject a `[turn:<id>]` token at the start of `text`.
    /// Returns the (possibly modified) text.
    pub fn inject_turn_ref(&self, text: &str, referenced_message_id: Option<&MessageId>) -> String {
        match referenced_message_id.and_then(|id| self.get(id)) {
            Some(turn_id) => {
                let token = turn_ref::construct(turn_id);
                format!("{token} {text}")
            }
            None => text.to_owned(),
        }
    }
}

impl Default for TurnMap {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn_id(bits: u128) -> TurnId {
        TurnId(ulid::Ulid::from(bits))
    }

    #[test]
    fn turn_map_inject_token() {
        let mut map = TurnMap::new();
        let msg_id = MessageId::new(123);
        let tid = turn_id(42);
        map.record(msg_id, tid);

        let text = "what did you mean by that?";
        let injected = map.inject_turn_ref(text, Some(&msg_id));
        let token = turn_ref::construct(tid);
        assert_eq!(injected, format!("{token} {text}"));
    }

    #[test]
    fn turn_map_miss_no_inject() {
        let map = TurnMap::new();
        let msg_id = MessageId::new(999);
        let text = "a normal message";
        let injected = map.inject_turn_ref(text, Some(&msg_id));
        assert_eq!(injected, text);
    }

    #[test]
    fn turn_map_none_reference_no_inject() {
        let map = TurnMap::new();
        let text = "a standalone message";
        let injected = map.inject_turn_ref(text, None);
        assert_eq!(injected, text);
    }
}
