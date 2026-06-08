//! The event envelope and the (deliberately growing) catalogue of event payloads.
//!
//! All state is events; graph state is a pure projection (spec §Event sourcing). Every payload
//! carries a `type` tag and a `version`, and the materializer dispatches on `(type, version)`. A
//! new capability adds a new variant or a higher version, and old logs replay unchanged —
//! extensibility without migrations. The content-, link-, visibility-, and time-bearing events
//! continue to arrive with the stages that need them (visibility at 6, time at 9).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::{
    ids::{
        ConversationId, ConversationLocator, EntryId, MemoryId, MemoryName, RelationName, Seq,
        SessionId, TagName, TurnId,
    },
    settings::Settings,
    time::{TemporalRef, Timestamp},
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

/// The author of a conversation turn (spec §Event sourcing → ConversationTurn). The participant and
/// session bindings arrive with the conversation machinery at Stage 8.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TurnRole {
    /// An inbound participant message.
    Participant,
    /// The agent's response cycle — exactly one per turn, however it ends.
    Agent,
    /// An injected system message (a join brief, a `<time_update/>`).
    System,
}

/// Whether a turn is the agent responding to a message or acting unprompted (spec §Time).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Initiation {
    Responding,
    Initiated,
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

/// The orchestration prompt templates the build ships — a closed, build-defined set (spec
/// §Initialization → prompt templates). Serialized in kebab-case to match the human-facing names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PromptTemplateName {
    /// The system-prompt scaffold.
    Scaffold,
    /// Synthesizes a memory's description from its entries.
    DescriptionRegen,
    /// Extracts temporal references from text.
    TemporalExtraction,
    /// Frames the pre-compaction flush turn: write durable working state to memory before the cut.
    Flush,
    /// Frames the control-panel imprint interview: meet the creator and form self-knowledge.
    Imprint,
}

impl PromptTemplateName {
    pub fn as_str(self) -> &'static str {
        match self {
            PromptTemplateName::Scaffold => "scaffold",
            PromptTemplateName::DescriptionRegen => "description-regen",
            PromptTemplateName::TemporalExtraction => "temporal-extraction",
            PromptTemplateName::Flush => "flush",
            PromptTemplateName::Imprint => "imprint",
        }
    }
}

/// Provenance for an event produced by model inference: the model and the prompt template (by name
/// and version) that wrote it (spec §Storage → provenance on inference). Carried by inference events
/// so "which model and template produced this" is answerable, and so regenerative replay knows what
/// to re-run; purely mechanical events leave it `None`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProducedBy {
    pub model_id: SmolStr,
    pub template_name: PromptTemplateName,
    pub template_version: u32,
}

/// Who told the agent a piece of content (spec §Visibility). Distinct from [`EventSource`], which is
/// authorship *authority*: `told_by` is the *teller* whose confidence the read-time predicate
/// reasons about.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Teller {
    /// A conversation participant, identified by their `person/*` memory.
    Participant(MemoryId),
    /// The agent's own observations and inferences. Defined as always present to itself.
    Agent,
    /// Genesis-seeded content, before any real teller exists.
    Bootstrap,
}

/// How widely a content entry may be surfaced (spec §Visibility). The read-time predicate
/// `visible(...)` interprets these against the present set; `PrivateToTeller` additionally never
/// surfaces to the subject of a person memory.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Visibility {
    /// Surfaces to any present set, including the subject.
    Public,
    /// Surfaces only while the teller is present, and never to the memory's subject.
    PrivateToTeller,
    /// As `PrivateToTeller`, additionally suppressed whenever any named party is present.
    Exclude(Vec<MemoryId>),
}

/// The data carried by an event, tagged by `type` on the wire. `Seq` and `recorded_at` live on the
/// [`Event`] envelope rather than here, because they are assigned by the store at append time.
///
/// Not `Eq`: [`Settings`] carries `f32` search weights. Equality is `PartialEq` throughout.
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
    /// Records a content entry. `told_by` is the teller, `told_in` the context it was told in (a
    /// `context/*` memory, resolved to its confidentiality at Stage 8; `None` until contexts exist),
    /// and `visibility` governs the read-time predicate. `asserted_at` is when the agent recorded the
    /// fact; `occurred_at` is the optional real-world time the fact is *about* (spec §Time →
    /// bi-temporality). `occurred_at` is `#[serde(default)]` so pre-Stage-9 logs, which lack the
    /// field, replay as `None`.
    MemoryContentAppended {
        id: MemoryId,
        entry_id: EntryId,
        asserted_at: Timestamp,
        #[serde(default)]
        occurred_at: Option<TemporalRef>,
        text: String,
        told_by: Teller,
        told_in: Option<MemoryId>,
        visibility: Visibility,
    },
    /// Resolves an entry's `occurred_at` after the fact: the turn-end extraction pass read the
    /// entry's natural language ("last Tuesday") and produced a structured [`TemporalRef`]. The
    /// original `MemoryContentAppended` stays immutable; applying this recomputes the entry's
    /// denormalized occurrence columns. `produced_by` records the extracting inference.
    EntryTemporalResolved {
        id: MemoryId,
        entry_id: EntryId,
        occurred_at: TemporalRef,
        produced_by: Option<ProducedBy>,
    },
    /// Replaces a memory's synthesized description. The text is produced by the model (Stage 5);
    /// applying it to the projection is purely mechanical. `produced_by` records the inference that
    /// wrote it (`None` only for a hand-seeded description).
    MemoryDescriptionRegenerated {
        id: MemoryId,
        new_text: String,
        produced_by: Option<ProducedBy>,
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
        name: PromptTemplateName,
        version: u32,
        body: String,
        source: EventSource,
    },
    /// Sets the behavioral tunables to a whole [`Settings`] snapshot. The current settings are the
    /// latest `ConfigSet`; defaults are seeded at genesis. Lives in the log so replay reproduces the
    /// behavior the values produced.
    ConfigSet {
        settings: Settings,
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
    /// A turn in the conversation: an inbound participant message, the agent's response (a reply, a
    /// silent terminal with empty `text`, or a surfaced `max_steps` error), or a system message.
    /// `participant` is the speaker of an inbound message (`None` for the agent's own and system
    /// turns). `produced_by` records the inference behind an `Agent` turn; participant and system
    /// turns are not inference, so it is `None`.
    ConversationTurn {
        conversation: ConversationId,
        turn_id: TurnId,
        role: TurnRole,
        text: String,
        participant: Option<MemoryId>,
        initiation: Initiation,
        produced_by: Option<ProducedBy>,
    },
    /// Opens a durable conversation (a room), keyed by its `locator`. Fires once on first contact;
    /// the room then persists across sessions for the agent's life (spec §Conversations).
    /// `context_memory` is the `context/*` memory minted eagerly alongside the room, so the locator
    /// resolves to a first-class memory the agent can tag (`#confidential`) and reason about.
    ConversationStarted {
        id: ConversationId,
        locator: ConversationLocator,
        context_memory: MemoryId,
    },
    /// Retires a conversation permanently — rare, since conversations are durable.
    ConversationEnded {
        id: ConversationId,
    },
    /// Opens a bounded activity window within a conversation — the brief-freeze unit.
    /// `participants` is the present set at open; `brief` is the composed brief captured verbatim so
    /// the frozen prompt is faithfully replayable (spec §System prompt → replay); `seeded_from_turn`
    /// records the carryover extent when the session opened via compaction (`None` for a fresh or
    /// idle-opened session).
    SessionStarted {
        conversation: ConversationId,
        id: SessionId,
        participants: Vec<MemoryId>,
        started_at: Timestamp,
        seeded_from_turn: Option<TurnId>,
        brief: String,
    },
    SessionEnded {
        conversation: ConversationId,
        id: SessionId,
    },
    /// A participant arriving mid-session, at turn `at_turn`.
    ParticipantJoined {
        conversation: ConversationId,
        session: SessionId,
        participant: MemoryId,
        at_turn: TurnId,
    },
    /// Binds a `person/*` stub to a platform identity, seeding the `(platform, platform_user_id) ->
    /// memory_id` operational mapping (spec §Identity). Emitted on first contact (with the
    /// `MemoryCreated` that mints the stub) and whenever an existing stub gains a further platform
    /// identity. The mapping is operational, not a memory-graph fact, so it lives in this event
    /// rather than as a relation.
    ParticipantIdentified {
        memory: MemoryId,
        platform: SmolStr,
        platform_user_id: SmolStr,
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
            EventPayload::EntryTemporalResolved { .. } => "EntryTemporalResolved",
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
            EventPayload::ConversationTurn { .. } => "ConversationTurn",
            EventPayload::ConversationStarted { .. } => "ConversationStarted",
            EventPayload::ConversationEnded { .. } => "ConversationEnded",
            EventPayload::SessionStarted { .. } => "SessionStarted",
            EventPayload::SessionEnded { .. } => "SessionEnded",
            EventPayload::ParticipantJoined { .. } => "ParticipantJoined",
            EventPayload::ParticipantIdentified { .. } => "ParticipantIdentified",
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
            | EventPayload::EntryTemporalResolved { id, .. }
            | EventPayload::MemoryDescriptionRegenerated { id, .. }
            | EventPayload::MemoryVolatilitySet { id, .. }
            | EventPayload::TagAppliedToMemory { memory: id, .. }
            | EventPayload::TagRemovedFromMemory { memory: id, .. }
            | EventPayload::LinkCreated { from: id, .. }
            | EventPayload::LinkRemoved { from: id, .. }
            | EventPayload::ParticipantIdentified { memory: id, .. } => Some(id.0.to_string()),
            EventPayload::TagCreated { name, .. }
            | EventPayload::TagDescriptionChanged { name, .. } => Some(name.as_str().to_owned()),
            EventPayload::LinkTypeRegistered { name, .. } => Some(name.as_str().to_owned()),
            EventPayload::PromptTemplateRegistered { name, .. } => Some(name.as_str().to_owned()),
            // A whole-settings snapshot, not about a single entity.
            EventPayload::ConfigSet { .. } => None,
            // Conversation-keyed events target the conversation, so per-conversation history (the
            // debugger's conversation view, compaction's read of a session's blocks) is a cheap
            // indexed filter. A `LuaExecuted` touches many memories, but it belongs to one
            // conversation; its memory set is recovered from `touched`, not this column.
            EventPayload::ConversationStarted { id, .. }
            | EventPayload::ConversationEnded { id } => Some(id.0.to_string()),
            EventPayload::LuaExecuted { conversation, .. }
            | EventPayload::ConversationTurn { conversation, .. }
            | EventPayload::SessionStarted { conversation, .. }
            | EventPayload::SessionEnded { conversation, .. }
            | EventPayload::ParticipantJoined { conversation, .. } => {
                Some(conversation.0.to_string())
            }
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

#[cfg(test)]
mod tests {
    use super::{EntryId, EventPayload, MemoryId, Teller, Timestamp, Visibility};
    use crate::time::{CivilDate, TemporalRef};

    fn content_with(occurred_at: Option<TemporalRef>) -> EventPayload {
        EventPayload::MemoryContentAppended {
            id: MemoryId::generate(),
            entry_id: EntryId::generate(),
            asserted_at: Timestamp::from_millis(1),
            occurred_at,
            text: "x".to_owned(),
            told_by: Teller::Agent,
            told_in: None,
            visibility: Visibility::Public,
        }
    }

    #[test]
    fn content_append_without_occurred_at_replays_as_none() {
        // A pre-Stage-9 payload predates the field; dropping the key models an old log. `serde(default)`
        // must fill `None` so the historical event deserializes unchanged.
        let mut value = serde_json::to_value(content_with(Some(TemporalRef::Day(CivilDate(
            "2026-06-03".into(),
        )))))
        .unwrap();
        value.as_object_mut().unwrap().remove("occurred_at");
        let replayed: EventPayload = serde_json::from_value(value).unwrap();
        assert!(matches!(
            replayed,
            EventPayload::MemoryContentAppended {
                occurred_at: None,
                ..
            }
        ));
    }

    #[test]
    fn content_append_round_trips_occurred_at() {
        let event = content_with(Some(TemporalRef::Instant(Timestamp::from_millis(42))));
        let json = serde_json::to_string(&event).unwrap();
        assert_eq!(serde_json::from_str::<EventPayload>(&json).unwrap(), event);
    }

    #[test]
    fn entry_temporal_resolved_round_trips() {
        let event = EventPayload::EntryTemporalResolved {
            id: MemoryId::generate(),
            entry_id: EntryId::generate(),
            occurred_at: TemporalRef::Day(CivilDate("2026-06-03".into())),
            produced_by: None,
        };
        let json = serde_json::to_string(&event).unwrap();
        assert_eq!(serde_json::from_str::<EventPayload>(&json).unwrap(), event);
    }
}
