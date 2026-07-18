//! Pacing: debounce rapid-fire messages into a single batch.
//!
//! The connector debounces rapid-fire messages per channel. Each new message resets the
//! debounce timer; when the timer fires, the whole queue is handed to a callback.

use std::{collections::HashMap, sync::Arc, time::Duration};

use parking_lot::Mutex;
use serenity::model::id::ChannelId;
use tokio::{task::JoinHandle, time::sleep};

/// One queued message waiting for the debounce window to clear.
pub struct PendingMessage {
    /// The message text (with `[turn:<id>]` injected if applicable).
    pub text: String,
    /// The sender's Discord user id (as a string, for the platform API).
    pub sender: String,
}

/// The fire callback for a channel's debounce. The latest one wins — each `submit` may
/// capture different per-message context (present set, locator, etc.).
type FireFn = Box<dyn FnOnce(Vec<PendingMessage>) + Send>;

/// Per-channel debounce state: a queue of pending messages, a fire callback, and a timer task.
/// Each new message aborts the old timer and starts a fresh one, so the batch fires `debounce`
/// after the *last* message, not the first.
struct ChannelDebounce {
    queue: Vec<PendingMessage>,
    on_fire: Option<FireFn>,
    timer: Option<JoinHandle<()>>,
}

/// Per-channel debounce. Holds a queue and a timer per channel.
pub struct DebounceState {
    channels: Mutex<HashMap<ChannelId, Arc<Mutex<ChannelDebounce>>>>,
    debounce: Duration,
}

impl DebounceState {
    pub fn new(debounce_ms: u64) -> Self {
        DebounceState {
            channels: Mutex::new(HashMap::new()),
            debounce: Duration::from_millis(debounce_ms),
        }
    }

    /// Enqueue a message and reset the debounce timer. When the timer fires (after `debounce`
    /// with no new message), `on_fire` is called with the full batch.
    pub fn submit<F>(&self, channel_id: ChannelId, msg: PendingMessage, on_fire: F)
    where
        F: FnOnce(Vec<PendingMessage>) + Send + 'static,
    {
        let mut channels = self.channels.lock();
        let entry = channels.entry(channel_id).or_insert_with(|| {
            Arc::new(Mutex::new(ChannelDebounce {
                queue: Vec::new(),
                on_fire: None,
                timer: None,
            }))
        });
        let mut debounce = entry.lock();
        debounce.queue.push(msg);
        debounce.on_fire = Some(Box::new(on_fire));
        // Abort the old timer — a new message resets the window.
        if let Some(handle) = debounce.timer.take() {
            handle.abort();
        }
        let debounce_dur = self.debounce;
        let entry_clone = entry.clone();
        let handle = tokio::spawn(async move {
            sleep(debounce_dur).await;
            let mut state = entry_clone.lock();
            let queue = std::mem::take(&mut state.queue);
            if let Some(fire) = state.on_fire.take() {
                fire(queue);
            }
        });
        debounce.timer = Some(handle);
    }
}
