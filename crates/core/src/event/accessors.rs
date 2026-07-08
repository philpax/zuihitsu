use super::EventPayload;

impl EventPayload {
    /// The `type` tag, used as the event-store `type` column and for `(type, version)` dispatch.
    pub fn kind(&self) -> &'static str {
        match self {
            EventPayload::GenesisCompleted { .. } => "GenesisCompleted",
            EventPayload::MemoryCreated { .. } => "MemoryCreated",
            EventPayload::MemoryRenamed { .. } => "MemoryRenamed",
            EventPayload::MemoryDeleted { .. } => "MemoryDeleted",
            EventPayload::MemoryContentAppended { .. } => "MemoryContentAppended",
            EventPayload::MemorySuperseded { .. } => "MemorySuperseded",
            EventPayload::EntryTemporalResolved { .. } => "EntryTemporalResolved",
            EventPayload::EntryTemporalResolveFailed { .. } => "EntryTemporalResolveFailed",
            EventPayload::EntryDescriptionMirrored { .. } => "EntryDescriptionMirrored",
            EventPayload::ScheduledJobFired { .. } => "ScheduledJobFired",
            EventPayload::ScheduledItemSurfaced { .. } => "ScheduledItemSurfaced",
            EventPayload::MemoryDescriptionRegenerated { .. } => "MemoryDescriptionRegenerated",
            EventPayload::BeliefArbitrated { .. } => "BeliefArbitrated",
            EventPayload::MergeProposed { .. } => "MergeProposed",
            EventPayload::MergeAdjudicated { .. } => "MergeAdjudicated",
            EventPayload::LinksInferred { .. } => "LinksInferred",
            EventPayload::DescribePassCompleted { .. } => "DescribePassCompleted",
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
            EventPayload::EmbeddingModelChanged { .. } => "EmbeddingModelChanged",
            EventPayload::LuaExecuted { .. } => "LuaExecuted",
            EventPayload::ModelCalled { .. } => "ModelCalled",
            EventPayload::ConversationTurn { .. } => "ConversationTurn",
            EventPayload::ConversationStarted { .. } => "ConversationStarted",
            EventPayload::ConversationEnded { .. } => "ConversationEnded",
            EventPayload::SessionStarted { .. } => "SessionStarted",
            EventPayload::SessionEnded { .. } => "SessionEnded",
            EventPayload::ParticipantJoined { .. } => "ParticipantJoined",
            EventPayload::ParticipantIdentified { .. } => "ParticipantIdentified",
        }
    }

    /// The payload-schema version; higher versions add fields. `MergeProposed` is `3` since gaining
    /// `source` (version 2) and then `rationale` (version 3); earlier payloads deserialize via those
    /// fields' defaults. Everything else is `1`.
    pub fn version(&self) -> u32 {
        match self {
            EventPayload::MergeProposed { .. } => 3,
            // Version 2 since gaining the structured `brief`; version-1 payloads replay via its default.
            EventPayload::ConversationTurn { .. } => 2,
            _ => 1,
        }
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
            | EventPayload::MemorySuperseded { id, .. }
            | EventPayload::EntryTemporalResolved { id, .. }
            | EventPayload::EntryTemporalResolveFailed { id, .. }
            | EventPayload::EntryDescriptionMirrored { id, .. }
            | EventPayload::MemoryDescriptionRegenerated { id, .. }
            | EventPayload::BeliefArbitrated { memory: id, .. }
            | EventPayload::MergeProposed { from: id, .. }
            | EventPayload::MergeAdjudicated { from: id, .. }
            | EventPayload::LinksInferred { memory: id, .. }
            | EventPayload::MemoryVolatilitySet { id, .. }
            | EventPayload::ScheduledJobFired { memory: id, .. }
            | EventPayload::ScheduledItemSurfaced { memory: id, .. }
            | EventPayload::TagAppliedToMemory { memory: id, .. }
            | EventPayload::TagRemovedFromMemory { memory: id, .. }
            | EventPayload::LinkCreated { from: id, .. }
            | EventPayload::LinkRemoved { from: id, .. }
            | EventPayload::ParticipantIdentified { memory: id, .. } => Some(id.0.to_string()),
            EventPayload::TagCreated { name, .. }
            | EventPayload::TagDescriptionChanged { name, .. } => Some(name.as_str().to_owned()),
            EventPayload::LinkTypeRegistered { name, .. } => Some(name.as_str().to_owned()),
            EventPayload::PromptTemplateRegistered { name, .. } => Some(name.as_str().to_owned()),
            // A whole-settings snapshot, a vector-index migration, and a describer pass over many
            // memories: none is about a single entity.
            EventPayload::ConfigSet { .. }
            | EventPayload::EmbeddingModelChanged { .. }
            | EventPayload::DescribePassCompleted { .. } => None,
            // Conversation-keyed events target the conversation, so per-conversation history (the
            // console's conversation view, compaction's read of a session's blocks) is a cheap
            // indexed filter. A `LuaExecuted` touches many memories, but it belongs to one
            // conversation; its memory set is recovered from `touched`, not this column.
            EventPayload::ConversationStarted { id, .. }
            | EventPayload::ConversationEnded { id } => Some(id.0.to_string()),
            EventPayload::LuaExecuted { conversation, .. }
            | EventPayload::ModelCalled { conversation, .. }
            | EventPayload::ConversationTurn { conversation, .. }
            | EventPayload::SessionStarted { conversation, .. }
            | EventPayload::SessionEnded { conversation, .. }
            | EventPayload::ParticipantJoined { conversation, .. } => {
                Some(conversation.0.to_string())
            }
        }
    }
}
