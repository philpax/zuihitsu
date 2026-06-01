//! The event envelope and the (deliberately growing) catalogue of event payloads.
//!
//! All state is events; graph state is a pure projection (spec §Event sourcing). Every payload
//! carries a `type` tag and a `version`, and the materializer dispatches on `(type, version)`. A
//! new capability adds a new variant or a higher version, and old logs replay unchanged —
//! extensibility without migrations. The content-, link-, visibility-, and time-bearing events
//! continue to arrive with the stages that need them (visibility at 6, time at 9).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::ids::{
    ConversationId, EntryId, MemoryId, MemoryName, RelationName, Seq, TagName, Timestamp, TurnId,
};

/// How sharply a memory's facts decay in search ranking (spec §Data model). Defaults to `Medium`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Volatility {
    Low,
    #[default]
    Medium,
    High,
}

impl Volatility {
    pub fn as_str(self) -> &'static str {
        match self {
            Volatility::Low => "Low",
            Volatility::Medium => "Medium",
            Volatility::High => "High",
        }
    }

    pub fn parse(text: &str) -> Option<Volatility> {
        match text {
            "Low" => Some(Volatility::Low),
            "Medium" => Some(Volatility::Medium),
            "High" => Some(Volatility::High),
            _ => None,
        }
    }
}

/// A relation endpoint's cardinality. `One` means a memory has at most one link of this relation
/// in that direction (enforcement of the replace-on-`One` rule is the Lua layer's, Stage 4).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Cardinality {
    One,
    Many,
}

impl Cardinality {
    pub fn as_str(self) -> &'static str {
        match self {
            Cardinality::One => "One",
            Cardinality::Many => "Many",
        }
    }

    pub fn parse(text: &str) -> Option<Cardinality> {
        match text {
            "One" => Some(Cardinality::One),
            "Many" => Some(Cardinality::Many),
            _ => None,
        }
    }
}

/// Who authored a link: the agent itself, or an operator acting through the control panel/debugger.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LinkSource {
    Agent,
    Debugger,
}

impl LinkSource {
    pub fn as_str(self) -> &'static str {
        match self {
            LinkSource::Agent => "Agent",
            LinkSource::Debugger => "Debugger",
        }
    }

    pub fn parse(text: &str) -> Option<LinkSource> {
        match text {
            "Agent" => Some(LinkSource::Agent),
            "Debugger" => Some(LinkSource::Debugger),
            _ => None,
        }
    }
}

/// Provenance for events that carry an authority, distinct from a participant teller: `Bootstrap`
/// for genesis, `Orchestration` for prompt templates, `Debugger` for operator/control writes, and
/// `Agent` for the agent's own (spec §Initialization, §Trust model).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventSource {
    Bootstrap,
    Agent,
    Debugger,
    Orchestration,
}

/// How a Lua block ended when the agent saw the outcome (spec §Event sourcing). A block that
/// commits normally has no terminal cause; one the agent observed failing or deliberately aborting
/// records why.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TerminalCause {
    /// A runtime error the agent saw, as its message.
    Error(String),
    /// An explicit `block.abort(reason)`.
    Aborted(String),
}

/// A behavioral tunable's value. Flat per-key scalars, never structured policy objects (spec
/// §Initialization → configuration).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ConfigValue {
    Int(i64),
    Float(f64),
    Text(String),
    Bool(bool),
}

/// The data carried by an event, tagged by `type` on the wire. `Seq` and `recorded_at` live on the
/// [`Event`] envelope rather than here, because they are assigned by the store at append time.
///
/// Not `Eq`: [`ConfigValue`] carries an `f64`. Equality is `PartialEq` throughout.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
    /// Records a content entry. Provenance and bi-temporality (told_by, visibility, occurred_at)
    /// are added at higher versions with the stages that introduce them (6, 9).
    MemoryContentAppended {
        id: MemoryId,
        entry_id: EntryId,
        asserted_at: Timestamp,
        text: String,
    },
    /// Replaces a memory's synthesized description. The text is produced by the model (Stage 5);
    /// applying it to the projection is purely mechanical.
    MemoryDescriptionRegenerated {
        id: MemoryId,
        new_text: String,
    },
    MemoryVolatilitySet {
        id: MemoryId,
        volatility: Volatility,
    },
    /// Creates a tag, which always forces a purpose. Distinct from application, which never mutates
    /// the description (spec §Lua API → tags).
    TagCreated {
        name: TagName,
        description: String,
    },
    TagDescriptionChanged {
        name: TagName,
        new_description: String,
    },
    TagAppliedToMemory {
        memory: MemoryId,
        tag: TagName,
    },
    TagRemovedFromMemory {
        memory: MemoryId,
        tag: TagName,
    },
    /// Registers a relation in the schema, accessible under either label; the inverse view's
    /// cardinality is computed (spec §Data model: the registry lives in data, not code).
    LinkTypeRegistered {
        name: RelationName,
        inverse: RelationName,
        from_card: Cardinality,
        to_card: Cardinality,
        symmetric: bool,
        reflexive: bool,
    },
    /// Creates a directed edge. The materializer canonicalizes direction at write time, so a link
    /// asserted under either label produces the same stored edge. `told_by` (for asymmetric-belief
    /// relations) arrives with identity at Stage 6/7.
    LinkCreated {
        from: MemoryId,
        to: MemoryId,
        relation: RelationName,
        source: LinkSource,
    },
    LinkRemoved {
        from: MemoryId,
        to: MemoryId,
        relation: RelationName,
    },
    /// Registers a versioned prompt template (scaffold, regen, …). Orchestration config, not
    /// agent-editable; updating a template is a new registration with a bumped version.
    PromptTemplateRegistered {
        name: String,
        version: u32,
        body: String,
        source: EventSource,
    },
    /// Sets a behavioral tunable. Current config is the latest `ConfigSet` per key; defaults are
    /// seeded at genesis. Lives in the log so replay reproduces the behavior the value produced.
    ConfigSet {
        key: String,
        value: ConfigValue,
        source: EventSource,
    },
    /// Records one executed Lua block — what the agent saw. The stored `result` is the value
    /// rendered back into the next inference step (text, not a live handle), so faithful replay
    /// feeds the model exactly the string it saw. `touched` is the set of memories the block read
    /// or wrote; `terminal_cause` is set only for agent-visible error/abort outcomes.
    LuaExecuted {
        conversation: ConversationId,
        turn_id: TurnId,
        script: String,
        result: Option<String>,
        touched: Vec<MemoryId>,
        terminal_cause: Option<TerminalCause>,
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
            EventPayload::MemoryContentAppended { .. } => "MemoryContentAppended",
            EventPayload::MemoryDescriptionRegenerated { .. } => "MemoryDescriptionRegenerated",
            EventPayload::MemoryVolatilitySet { .. } => "MemoryVolatilitySet",
            EventPayload::TagCreated { .. } => "TagCreated",
            EventPayload::TagDescriptionChanged { .. } => "TagDescriptionChanged",
            EventPayload::TagAppliedToMemory { .. } => "TagAppliedToMemory",
            EventPayload::TagRemovedFromMemory { .. } => "TagRemovedFromMemory",
            EventPayload::LinkTypeRegistered { .. } => "LinkTypeRegistered",
            EventPayload::LinkCreated { .. } => "LinkCreated",
            EventPayload::LinkRemoved { .. } => "LinkRemoved",
            EventPayload::PromptTemplateRegistered { .. } => "PromptTemplateRegistered",
            EventPayload::ConfigSet { .. } => "ConfigSet",
            EventPayload::LuaExecuted { .. } => "LuaExecuted",
        }
    }

    /// The payload-schema version. All current payloads are `1`; higher versions add fields.
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
            | EventPayload::MemoryDeleted { id }
            | EventPayload::MemoryContentAppended { id, .. }
            | EventPayload::MemoryDescriptionRegenerated { id, .. }
            | EventPayload::MemoryVolatilitySet { id, .. }
            | EventPayload::TagAppliedToMemory { memory: id, .. }
            | EventPayload::TagRemovedFromMemory { memory: id, .. }
            | EventPayload::LinkCreated { from: id, .. }
            | EventPayload::LinkRemoved { from: id, .. } => Some(id.0.to_string()),
            EventPayload::TagCreated { name, .. }
            | EventPayload::TagDescriptionChanged { name, .. } => Some(name.as_str().to_owned()),
            EventPayload::LinkTypeRegistered { name, .. } => Some(name.as_str().to_owned()),
            EventPayload::PromptTemplateRegistered { name, .. } => Some(name.clone()),
            EventPayload::ConfigSet { key, .. } => Some(key.clone()),
            // Touches many memories rather than one; recoverable from its `touched` set, not a
            // single target column.
            EventPayload::LuaExecuted { .. } => None,
        }
    }
}

/// A committed event: a payload assigned a position in the log's total order and stamped with the
/// wall-clock time it was recorded. This is what a read returns; it is immutable.
#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub seq: Seq,
    pub recorded_at: Timestamp,
    pub payload: EventPayload,
}
