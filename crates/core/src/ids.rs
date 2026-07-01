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
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Seq(#[cfg_attr(feature = "ts", ts(type = "number"))] pub u64);

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
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct MemoryId(#[cfg_attr(feature = "ts", ts(type = "string"))] pub Ulid);

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
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ConversationId(#[cfg_attr(feature = "ts", ts(type = "string"))] pub Ulid);

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
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ConversationLocator {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub platform: SmolStr,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
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
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SessionId(#[cfg_attr(feature = "ts", ts(type = "string"))] pub Ulid);

impl SessionId {
    pub fn generate() -> SessionId {
        SessionId(Ulid::new())
    }
}

/// One run of the agent loop — a whole response cycle, producing exactly one `role = agent`
/// turn. A block's buffered side effects and its `LuaExecuted` share their turn's id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct TurnId(#[cfg_attr(feature = "ts", ts(type = "string"))] pub Ulid);

impl TurnId {
    pub fn generate() -> TurnId {
        TurnId(Ulid::new())
    }
}

/// The stable, globally-unique identity of a single content entry — addressable for supersession,
/// arbitration references, and per-entry vectors.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct EntryId(#[cfg_attr(feature = "ts", ts(type = "string"))] pub Ulid);

impl EntryId {
    pub fn generate() -> EntryId {
        EntryId(Ulid::new())
    }
}

/// A memory's agent-facing handle, namespaced by kind (e.g. `person/dave`, `topic/climbing`).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct MemoryName(#[cfg_attr(feature = "ts", ts(type = "string"))] pub SmolStr);

impl MemoryName {
    /// The reserved handle of the agent's self-model memory: seeded at genesis, and writable only
    /// from the console under operator authority (see the main crate's `memory::memory_block::Authority`).
    /// Held here so the one literal has a single home, used wherever code looks `self` up or guards a
    /// write against it.
    pub const SELF: &'static str = "self";

    pub fn new(name: impl Into<SmolStr>) -> MemoryName {
        MemoryName(name.into())
    }

    /// The reserved self handle as a value — `self` is in no namespace, so it has no
    /// [`NamespacedMemoryName`] form; this is the typed handle to pass where one is wanted (a lookup),
    /// distinct from the [`MemoryName::SELF`] string used for comparison.
    pub fn self_handle() -> MemoryName {
        MemoryName::new(MemoryName::SELF)
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Whether this is the reserved [`MemoryName::SELF`] handle.
    pub fn is_self(&self) -> bool {
        self.0 == MemoryName::SELF
    }

    /// The typed decomposition of this handle into its namespace and subject, if it is in a known
    /// namespace. The reserved `self` handle is in none, so it returns [`UnknownNamespace`].
    pub fn namespaced(&self) -> Result<NamespacedMemoryName, UnknownNamespace> {
        self.as_str().parse()
    }
}

/// The parse error for a handle that is in no known namespace (e.g. the reserved `self`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnknownNamespace;

/// A handle decomposed into its namespace and subject (`person/dave` ⇄ [`Namespace::Person`] +
/// `"dave"`) — the typed form of a [`MemoryName`]. Construction and parsing route through here so
/// the prefix concatenation has a single home and handles are never assembled from a literal.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct NamespacedMemoryName {
    pub namespace: Namespace,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub subject: SmolStr,
}

impl NamespacedMemoryName {
    pub fn new(namespace: Namespace, subject: impl Into<SmolStr>) -> NamespacedMemoryName {
        NamespacedMemoryName {
            namespace,
            subject: subject.into(),
        }
    }

    /// The operator's reserved identity anchor (`person/operator`): minted on first imprint and
    /// merged (`same_as`) into the operator's real `person/<name>` profile. It carries no content of
    /// its own — facts about the operator belong on their real profile — so a write to it is guarded
    /// against, leaving it a pure anchor for the merge. Built here so the one handle has a single
    /// home, used wherever the operator is resolved or a write is guarded against it.
    pub fn operator() -> NamespacedMemoryName {
        Namespace::Person.with_name("operator")
    }
}

impl From<NamespacedMemoryName> for MemoryName {
    fn from(name: NamespacedMemoryName) -> MemoryName {
        MemoryName::new(format!("{}{}", name.namespace.prefix(), name.subject))
    }
}

impl From<&NamespacedMemoryName> for MemoryName {
    fn from(name: &NamespacedMemoryName) -> MemoryName {
        MemoryName::new(format!("{}{}", name.namespace.prefix(), name.subject))
    }
}

impl From<&MemoryName> for MemoryName {
    fn from(name: &MemoryName) -> MemoryName {
        name.clone()
    }
}

impl std::fmt::Display for NamespacedMemoryName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}{}", self.namespace.prefix(), self.subject)
    }
}

impl std::str::FromStr for NamespacedMemoryName {
    type Err = UnknownNamespace;

    fn from_str(handle: &str) -> Result<NamespacedMemoryName, UnknownNamespace> {
        for ns in Namespace::ALL {
            if let Some(subject) = handle.strip_prefix(ns.prefix()) {
                return Ok(NamespacedMemoryName::new(ns, subject));
            }
        }
        Err(UnknownNamespace)
    }
}

/// The kinds of memory, each its own handle namespace (`person/dave`, `event/wedding`, …). The single
/// home for the prefix strings — like [`MemoryName::SELF`] for the reserved self handle — so adding a
/// kind or renaming a prefix is one edit here, and every handle is built by concatenating through
/// [`Namespace::with_name`] rather than from a literal scattered across the code.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum Namespace {
    Person,
    Place,
    Event,
    Topic,
    Context,
}

impl Namespace {
    /// Every namespace, in the order the scaffold introduces them — so a definition that enumerates the
    /// kinds (the genesis scaffold, the console legend) iterates this rather than re-listing prefixes.
    pub const ALL: [Namespace; 5] = [
        Namespace::Person,
        Namespace::Place,
        Namespace::Event,
        Namespace::Topic,
        Namespace::Context,
    ];

    /// The handle prefix, trailing slash included (`person/`).
    pub const fn prefix(self) -> &'static str {
        match self {
            Namespace::Person => "person/",
            Namespace::Place => "place/",
            Namespace::Event => "event/",
            Namespace::Topic => "topic/",
            Namespace::Context => "context/",
        }
    }

    /// Name a subject in this namespace, yielding the typed form: `Namespace::Person.with_name("dave")`
    /// is [`Namespace::Person`] + `"dave"`, which converts to the `person/dave` handle.
    pub fn with_name(self, subject: impl Into<SmolStr>) -> NamespacedMemoryName {
        NamespacedMemoryName::new(self, subject)
    }
}

#[cfg(test)]
mod tests {
    use super::{MemoryName, Namespace, NamespacedMemoryName, UnknownNamespace};

    #[test]
    fn round_trips_through_the_handle() {
        let typed = Namespace::Person.with_name("dave");
        let handle = MemoryName::from(&typed);
        assert_eq!(handle.as_str(), "person/dave");

        let parsed = handle.namespaced().unwrap();
        assert_eq!(parsed.namespace, Namespace::Person);
        assert_eq!(parsed.subject.as_str(), "dave");
        assert_eq!(parsed, typed);
    }

    #[test]
    fn the_self_handle_is_in_no_namespace() {
        assert_eq!(
            MemoryName::SELF.parse::<NamespacedMemoryName>(),
            Err(UnknownNamespace)
        );
    }
}
