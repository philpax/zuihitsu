//! Core identifier and value newtypes shared across the event log and (later) the materialized
//! graph. Two-tier identity (see spec §Data model): internal references use the immutable ULID,
//! agent-facing references use the mutable name, so a memory can be renamed without breaking links.

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;
use ulid::Ulid;

/// A position in the event log's single total order. The first event is `Seq(1)`; `Seq::ZERO`
/// denotes "before any event" and is the lower bound for a full read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Seq(pub u64);

impl Seq {
    /// The position before the first event. `read_from(Seq::ZERO)` returns the whole log.
    pub const ZERO: Seq = Seq(0);

    /// The next position in the total order.
    pub fn next(self) -> Seq {
        Seq(self.0 + 1)
    }
}

/// Wall-clock time as milliseconds since the Unix epoch, UTC. A denormalized convenience for
/// human-readable queries and recency math; `Seq` is the authoritative timeline, and `Seq` breaks
/// ties (see spec §Time → sequence vs wall-clock).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Timestamp(pub i64);

impl Timestamp {
    pub fn from_millis(millis: i64) -> Timestamp {
        Timestamp(millis)
    }

    pub fn as_millis(self) -> i64 {
        self.0
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
    pub fn new(name: impl Into<SmolStr>) -> MemoryName {
        MemoryName(name.into())
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

/// A tag's unique name.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TagName(pub SmolStr);

impl TagName {
    pub fn new(name: impl Into<SmolStr>) -> TagName {
        TagName(name.into())
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

/// A link relation's canonical name (e.g. `mentor_of`). One relation has two labels — itself and
/// its inverse — and the materializer canonicalizes to this name (spec §Data model: link relation).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RelationName(pub SmolStr);

impl RelationName {
    pub fn new(name: impl Into<SmolStr>) -> RelationName {
        RelationName(name.into())
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}
