//! The event envelope and the (small, deliberately growing) catalogue of event payloads.
//!
//! All state is events; graph state is a pure projection (spec §Event sourcing). Every payload
//! carries a `type` tag and a `version`, and the materializer (Stage 2) dispatches on
//! `(type, version)`. A new capability adds a new variant or a higher version, and old logs replay
//! unchanged — extensibility without migrations. The set below is the Stage 1 core; the
//! content-, visibility-, and time-bearing events arrive with the stages that need them.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::ids::{MemoryId, MemoryName, Seq, TagName, Timestamp};

/// The data carried by an event, tagged by `type` on the wire. `Seq` and `recorded_at` live on the
/// [`Event`] envelope rather than here, because they are assigned by the store at append time.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum EventPayload {
    /// Marks a completed genesis sequence; boot branches on its presence, not on log emptiness.
    GenesisCompleted {
        manifest_hash: String,
        template_versions: BTreeMap<String, u32>,
    },
    /// Creates an empty memory. Initial content is recorded as a paired content-append event, so
    /// there is exactly one provenance path for all content.
    MemoryCreated {
        id: MemoryId,
        name: MemoryName,
    },
    MemoryRenamed {
        id: MemoryId,
        old_name: MemoryName,
        new_name: MemoryName,
    },
    /// Soft delete: contents are preserved for replay and audit; the projection sets a flag.
    MemoryDeleted {
        id: MemoryId,
    },
    TagCreated {
        name: TagName,
        description: String,
    },
}

impl EventPayload {
    /// The `type` tag, used as the event-store `type` column and for `(type, version)` dispatch.
    pub fn kind(&self) -> &'static str {
        match self {
            EventPayload::GenesisCompleted { .. } => "GenesisCompleted",
            EventPayload::MemoryCreated { .. } => "MemoryCreated",
            EventPayload::MemoryRenamed { .. } => "MemoryRenamed",
            EventPayload::MemoryDeleted { .. } => "MemoryDeleted",
            EventPayload::TagCreated { .. } => "TagCreated",
        }
    }

    /// The payload-schema version. All Stage 1 payloads are `1`; higher versions add fields.
    pub fn version(&self) -> u32 {
        1
    }

    /// The primary entity this event is about, stored as an indexed column so per-target history is
    /// a cheap filter (spec §Event sourcing: per-memory history). `None` for log-wide events.
    pub fn target_id(&self) -> Option<String> {
        match self {
            EventPayload::GenesisCompleted { .. } => None,
            EventPayload::MemoryCreated { id, .. }
            | EventPayload::MemoryRenamed { id, .. }
            | EventPayload::MemoryDeleted { id } => Some(id.0.to_string()),
            EventPayload::TagCreated { name, .. } => Some(name.as_str().to_owned()),
        }
    }
}

/// A committed event: a payload assigned a position in the log's total order and stamped with the
/// wall-clock time it was recorded. This is what a read returns; it is immutable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    pub seq: Seq,
    pub recorded_at: Timestamp,
    pub payload: EventPayload,
}
