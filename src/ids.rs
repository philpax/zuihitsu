//! Core identifier and value newtypes shared across the event log and (later) the materialized
//! graph. Two-tier identity (see spec §Data model): internal references use the immutable ULID,
//! agent-facing references use the mutable name, so a memory can be renamed without breaking links.

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;
use ulid::Ulid;

/// A position in the event log's single total order. The first event is `Seq(1)`; `Seq::ZERO`
/// denotes "before any event" and is the lower bound for a full read.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct Seq(pub u64);

impl Seq {
    /// The position before the first event. `read_from(Seq::ZERO)` returns the whole log.
    pub const ZERO: Seq = Seq(0);

    /// The next position in the total order.
    pub fn next(self) -> Seq {
        Seq(self.0 + 1)
    }
}

/// The canonical, immutable, internal identity of a memory.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MemoryId(pub Ulid);

impl MemoryId {
    /// Mint a fresh identity. ULIDs are time-ordered and globally unique; the minted value is
    /// recorded in the log and read back verbatim on replay, so generation is not itself replayed.
    pub fn generate() -> MemoryId {
        MemoryId(Ulid::new())
    }
}

/// A durable conversation (a room the agent meets again and again), keyed by its
/// [`ConversationLocator`] and persisting across sessions for the agent's life.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ConversationId(pub Ulid);

impl ConversationId {
    pub fn generate() -> ConversationId {
        ConversationId(Ulid::new())
    }
}

/// The stable address of a durable conversation on a platform — what a platform client reports so
/// the server resolves it to the same [`ConversationId`] every time. `platform` is a short config
/// key (`direct`, `discord`, `slack`); `scope_path` locates the room within it (a channel id, a DM
/// thread). Two locators name the same room exactly when both fields match.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ConversationLocator {
    pub platform: SmolStr,
    pub scope_path: SmolStr,
}

impl ConversationLocator {
    pub fn new(
        platform: impl Into<SmolStr>,
        scope_path: impl Into<SmolStr>,
    ) -> ConversationLocator {
        ConversationLocator {
            platform: platform.into(),
            scope_path: scope_path.into(),
        }
    }
}

/// One bounded activity window within a conversation — the unit that freezes a brief and anchors the
/// prefix cache. Opens on first activity (or resumption after a quiet gap, or a compaction
/// re-segment) and closes on idle (spec §Conversations).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SessionId(pub Ulid);

impl SessionId {
    pub fn generate() -> SessionId {
        SessionId(Ulid::new())
    }
}

/// One run of the agent loop — a whole response cycle, producing exactly one `role = agent`
/// turn. A block's buffered side effects and its `LuaExecuted` share their turn's id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TurnId(pub Ulid);

impl TurnId {
    pub fn generate() -> TurnId {
        TurnId(Ulid::new())
    }
}

/// The stable, globally-unique identity of a single content entry — addressable for supersession,
/// arbitration references, and per-entry vectors.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EntryId(pub Ulid);

impl EntryId {
    pub fn generate() -> EntryId {
        EntryId(Ulid::new())
    }
}

/// A memory's agent-facing handle, namespaced by kind (e.g. `person/dave`, `topic/climbing`).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MemoryName(pub SmolStr);

impl MemoryName {
    /// The reserved handle of the agent's self-model memory: seeded at genesis, and writable only
    /// from the control panel (see [`crate::memory::memory_block::Authority`]). Held here so the one literal
    /// has a single home, used wherever code looks `self` up or guards a write against it.
    pub const SELF: &'static str = "self";

    pub fn new(name: impl Into<SmolStr>) -> MemoryName {
        MemoryName(name.into())
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Whether this is the reserved [`MemoryName::SELF`] handle.
    pub fn is_self(&self) -> bool {
        self.0 == MemoryName::SELF
    }
}
