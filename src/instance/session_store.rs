//! The live session map and its lifecycle/carryover state — pure runtime state, never logged.

use std::{collections::HashMap, sync::Arc};

use parking_lot::Mutex;

use crate::ids::{ConversationId, SessionId};

use super::session::{Carryover, OpenSession};

/// The live session map and its lifecycle/carryover state — pure runtime state, never logged.
/// Each session map entry is an `Arc` so a turn holds its session across the turn `.await` without
/// keeping the map guard. The lifecycle map mints a per-conversation async lock serializing the
/// session lifecycle (close-with-flush then open). The carryover map stages a compacted session's
/// tail for the next `ensure_session` to seed.
pub(crate) struct SessionStore {
    /// The live session per conversation: its id, the VM whose globals persist across the session's
    /// turns, the frozen brief, and the last-activity time the idle-gap is measured from. Pure
    /// runtime state — never logged (the `SessionStarted` / `SessionEnded` events are); an agent
    /// restart drops the map, but the next message recovers a session still open in the log through
    /// `ensure_session` (resumed within the idle gap, else closed-with-flush and reopened). Behind a
    /// `Mutex` (and each value an `Arc`) so concurrent conversations reach the map through a shared
    /// `&Instance`; a turn holds its session's `Arc` across the turn `.await` without keeping the map guard.
    sessions: Mutex<HashMap<ConversationId, Arc<OpenSession>>>,
    /// A per-conversation async lock serializing its session lifecycle: the close-with-flush of one
    /// session and the open of the next. A close runs a flush — a model call lasting seconds — before it
    /// records `SessionEnded`, and within that window the idle sweep and the message-driven recovery path
    /// both reach the close for the same session. Held across `ensure_session` and the sweep's close, it
    /// makes the message path *wait* for an in-flight sweep close to finish before opening the next
    /// session — so that session's brief reads the flush's writes — and lets the second closer see the
    /// session already ended and skip. Locks are minted lazily and kept (one per conversation the agent
    /// ever holds; negligible).
    lifecycle: Mutex<HashMap<ConversationId, Arc<tokio::sync::Mutex<()>>>>,
    /// Carryover staged by a token-triggered compaction, consumed by the next `ensure_session` to
    /// seed the re-segmented session (spec §Compaction). Keyed by conversation; an entry lives only
    /// between the compacting turn and the next message in that room. Behind a `Mutex` for the same
    /// shared-`&Instance` reason as `sessions`.
    pending_carryover: Mutex<HashMap<ConversationId, Carryover>>,
}

impl SessionStore {
    /// Construct an empty store.
    pub(crate) fn new() -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            lifecycle: Mutex::new(HashMap::new()),
            pending_carryover: Mutex::new(HashMap::new()),
        }
    }

    /// Get the live session for a conversation, if any.
    pub(crate) fn get(&self, conversation: ConversationId) -> Option<Arc<OpenSession>> {
        self.sessions.lock().get(&conversation).cloned()
    }

    /// Insert or replace the session for a conversation.
    pub(crate) fn insert(&self, conversation: ConversationId, open: Arc<OpenSession>) {
        self.sessions.lock().insert(conversation, open);
    }

    /// Remove and return the session for a conversation, if any.
    pub(crate) fn remove(&self, conversation: ConversationId) -> Option<Arc<OpenSession>> {
        self.sessions.lock().remove(&conversation)
    }

    /// Remove the session for a conversation only if its id matches `expected`, returning it.
    /// Used by the idle sweep to atomically get-then-conditionally-remove under one lock: the
    /// `lifecycle_lock` serializes the conversation's lifecycle, so a split is race-safe, but the
    /// compound method keeps the intent legible.
    pub(crate) fn remove_if_matches(
        &self,
        conversation: ConversationId,
        expected: SessionId,
    ) -> Option<Arc<OpenSession>> {
        let mut sessions = self.sessions.lock();
        if sessions
            .get(&conversation)
            .is_some_and(|s| s.id == expected)
        {
            sessions.remove(&conversation)
        } else {
            None
        }
    }

    /// The number of live sessions, for the control facet's active-session gauge.
    pub(crate) fn active_count(&self) -> usize {
        self.sessions.lock().len()
    }

    /// Every live session with its conversation, collected under a single lock acquisition — the
    /// checkpoint sweeper's candidate list (and its audience gate's view of who else is active).
    pub(crate) fn live(&self) -> Vec<(ConversationId, Arc<OpenSession>)> {
        self.sessions
            .lock()
            .iter()
            .map(|(conversation, open)| (*conversation, open.clone()))
            .collect()
    }

    /// Drain all live sessions for shutdown, collecting them under a single lock acquisition.
    pub(crate) fn drain(&self) -> Vec<Arc<OpenSession>> {
        self.sessions
            .lock()
            .drain()
            .map(|(_, session)| session)
            .collect()
    }

    /// The lazily-minted async lock serializing `conversation`'s session lifecycle. Acquired across
    /// `ensure_session` and the idle sweep's close, so the close-with-flush of one session always
    /// finishes before the next session for that conversation opens.
    pub(crate) fn lifecycle_lock(
        &self,
        conversation: ConversationId,
    ) -> Arc<tokio::sync::Mutex<()>> {
        self.lifecycle
            .lock()
            .entry(conversation)
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    }

    /// Take and return the pending carryover for a conversation, if any (consumed by
    /// `ensure_session`).
    pub(crate) fn take_carryover(&self, conversation: ConversationId) -> Option<Carryover> {
        self.pending_carryover.lock().remove(&conversation)
    }

    /// Stage a carryover for a conversation (set by `Platform::end_session_for_compaction`).
    pub(crate) fn insert_carryover(&self, conversation: ConversationId, carryover: Carryover) {
        self.pending_carryover
            .lock()
            .insert(conversation, carryover);
    }
}
