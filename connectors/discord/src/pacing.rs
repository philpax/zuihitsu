//! Pacing and typing: debounce, queue coalescing, and typing indicator management.
//!
//! The connector debounces rapid-fire messages (resetting a timer per channel), coalesces pending
//! messages (only the latest unprocessed message per channel is kept), and shows a typing indicator
//! only after the agent begins emitting reply tokens — not during deliberation.

use std::{collections::HashMap, time::Duration};

use serenity::model::id::ChannelId;
use tokio::{sync::Mutex, time::Instant};

/// The pending state for one channel: the latest message waiting for the debounce to fire.
pub struct PendingMessage {
    /// The Discord message id.
    #[allow(dead_code)]
    pub message_id: u64,
    /// The message text (with `[turn:<id>]` injected if applicable).
    pub text: String,
    /// The sender's Discord user id (as a string, for the platform API).
    pub sender: String,
    /// Whether this is a direct address (DM or mention).
    #[allow(dead_code)]
    pub is_direct: bool,
}

/// Per-channel debounce state. Tracks the latest pending message and when the debounce fires.
pub struct DebounceState {
    pending: Mutex<HashMap<ChannelId, (Instant, PendingMessage)>>,
    debounce: Duration,
}

impl DebounceState {
    pub fn new(debounce_ms: u64) -> Self {
        DebounceState {
            pending: Mutex::new(HashMap::new()),
            debounce: Duration::from_millis(debounce_ms),
        }
    }

    /// Submit a message for debounced processing. Returns the instant the debounce fires, or `None`
    /// if a newer message should take precedence (the caller should replace, not stack). In practice,
    /// the caller always replaces: the latest message per channel wins.
    pub async fn submit(&self, channel_id: ChannelId, msg: PendingMessage) -> Instant {
        let fire_at = Instant::now() + self.debounce;
        let mut pending = self.pending.lock().await;
        pending.insert(channel_id, (fire_at, msg));
        fire_at
    }

    /// Take the pending message for a channel (called when the debounce timer fires).
    pub async fn take(&self, channel_id: ChannelId) -> Option<PendingMessage> {
        let mut pending = self.pending.lock().await;
        pending.remove(&channel_id).map(|(_, msg)| msg)
    }
}
