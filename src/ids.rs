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

/// A tag's name. Like [`RelationName`], the build's meaningful tags are named variants code can
/// match — `Confidential` drives the room-confidentiality marker (see spec §Visibility → marker) —
/// and everything else falls to `Other`. It serializes as its bare name, so the wire format is just
/// the string.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TagName {
    Confidential,
    Other(SmolStr),
}

impl TagName {
    /// Recognize a tag name, mapping a build-meaningful tag to its variant and anything else (an
    /// agent- or operator-created tag) to [`TagName::Other`].
    pub fn new(name: impl Into<SmolStr>) -> TagName {
        let name = name.into();
        match name.as_str() {
            "confidential" => TagName::Confidential,
            _ => TagName::Other(name),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            TagName::Confidential => "confidential",
            TagName::Other(name) => name.as_str(),
        }
    }
}

impl Serialize for TagName {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for TagName {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<TagName, D::Error> {
        Ok(TagName::new(SmolStr::deserialize(deserializer)?))
    }
}

/// A link relation, by label. The relation registry lives in data (spec §Data model) and the agent
/// registers relations at runtime, so this is a typed lens over the names: the build's seed
/// relations are named variants that code can match (`SameAs` drives identity-class merging,
/// `ActiveIn` the compaction carryover), and everything else — including the inverse labels — falls
/// to `Other`. It serializes as its bare name, so the wire format is just the string.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RelationName {
    CreatedBy,
    OperatorOf,
    Knows,
    SameAs,
    ActiveIn,
    /// The inverse label of [`RelationName::CreatedBy`].
    Created,
    /// The inverse label of [`RelationName::OperatorOf`].
    Operates,
    /// The inverse label of [`RelationName::Knows`].
    KnownBy,
    /// The inverse label of [`RelationName::ActiveIn`].
    HasActive,
    Other(SmolStr),
}

impl RelationName {
    /// Recognize a label, mapping a seed relation — or its inverse — to its variant and anything
    /// else (a runtime-registered relation) to [`RelationName::Other`]. [`RelationName::SameAs`] is
    /// its own inverse, so it has no separate variant.
    pub fn new(name: impl Into<SmolStr>) -> RelationName {
        let name = name.into();
        match name.as_str() {
            "created_by" => RelationName::CreatedBy,
            "operator_of" => RelationName::OperatorOf,
            "knows" => RelationName::Knows,
            "same_as" => RelationName::SameAs,
            "active_in" => RelationName::ActiveIn,
            "created" => RelationName::Created,
            "operates" => RelationName::Operates,
            "known_by" => RelationName::KnownBy,
            "has_active" => RelationName::HasActive,
            _ => RelationName::Other(name),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            RelationName::CreatedBy => "created_by",
            RelationName::OperatorOf => "operator_of",
            RelationName::Knows => "knows",
            RelationName::SameAs => "same_as",
            RelationName::ActiveIn => "active_in",
            RelationName::Created => "created",
            RelationName::Operates => "operates",
            RelationName::KnownBy => "known_by",
            RelationName::HasActive => "has_active",
            RelationName::Other(name) => name.as_str(),
        }
    }
}

impl Serialize for RelationName {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for RelationName {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<RelationName, D::Error> {
        Ok(RelationName::new(SmolStr::deserialize(deserializer)?))
    }
}
