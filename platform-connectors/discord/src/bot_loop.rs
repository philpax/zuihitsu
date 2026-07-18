//! Bot-loop guard: caps the number of consecutive turns another bot may initiate in a channel, so
//! two agents answering each other cannot ping-pong forever.
//!
//! Only *other* bots are counted — the connector never processes its own messages, so from its
//! vantage a bot-to-bot exchange appears as a run of other-bot messages with no human message
//! between them. A human speaking in the channel breaks the run and clears the cap.

use std::collections::HashMap;

use parking_lot::Mutex;
use serenity::model::id::ChannelId;

/// Per-channel guard against runaway bot-to-bot exchanges.
pub struct BotLoopGuard {
    /// The cap on consecutive bot-initiated turns per channel. Once a channel has fired this many
    /// in a row with no human message between them, further other-bot messages are dropped until a
    /// human speaks.
    max_consecutive: u32,
    /// Per-channel count of consecutive other-bot messages admitted since the last human message. A
    /// channel absent from the map has a streak of zero.
    streaks: Mutex<HashMap<ChannelId, u32>>,
}

impl BotLoopGuard {
    pub fn new(max_consecutive: u32) -> Self {
        BotLoopGuard {
            max_consecutive,
            streaks: Mutex::new(HashMap::new()),
        }
    }

    /// Note that a human spoke in the channel, clearing its bot-to-bot streak.
    pub fn note_human(&self, channel_id: ChannelId) {
        self.streaks.lock().remove(&channel_id);
    }

    /// Admit an other-bot message. Returns `false` once the channel's streak has reached the cap —
    /// the caller drops the message — and otherwise counts it and returns `true`. A cap of zero
    /// admits no other-bot message at all.
    pub fn admit_bot(&self, channel_id: ChannelId) -> bool {
        let mut streaks = self.streaks.lock();
        let count = streaks.entry(channel_id).or_insert(0);
        if *count >= self.max_consecutive {
            return false;
        }
        *count += 1;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admits_up_to_the_cap_then_drops() {
        let guard = BotLoopGuard::new(3);
        let channel = ChannelId::new(1);
        assert!(guard.admit_bot(channel));
        assert!(guard.admit_bot(channel));
        assert!(guard.admit_bot(channel));
        // The fourth consecutive bot message trips the guard.
        assert!(!guard.admit_bot(channel));
        assert!(!guard.admit_bot(channel));
    }

    #[test]
    fn a_human_clears_the_streak() {
        let guard = BotLoopGuard::new(2);
        let channel = ChannelId::new(1);
        assert!(guard.admit_bot(channel));
        assert!(guard.admit_bot(channel));
        assert!(!guard.admit_bot(channel));
        // A human message resets the count, so bots are admitted afresh.
        guard.note_human(channel);
        assert!(guard.admit_bot(channel));
        assert!(guard.admit_bot(channel));
        assert!(!guard.admit_bot(channel));
    }

    #[test]
    fn streaks_are_per_channel() {
        let guard = BotLoopGuard::new(1);
        let a = ChannelId::new(1);
        let b = ChannelId::new(2);
        assert!(guard.admit_bot(a));
        // The cap on channel `a` does not touch channel `b`.
        assert!(guard.admit_bot(b));
        assert!(!guard.admit_bot(a));
        assert!(!guard.admit_bot(b));
    }

    #[test]
    fn a_zero_cap_admits_nothing() {
        let guard = BotLoopGuard::new(0);
        assert!(!guard.admit_bot(ChannelId::new(1)));
    }
}
