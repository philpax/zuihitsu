use std::collections::BTreeMap;

use smol_str::SmolStr;

use crate::{
    ids::{ConversationId, ConversationLocator, EntryId, MemoryId, MemoryName, SessionId, TurnId},
    settings::Settings,
    time::{TemporalRef, Timestamp},
    vocabulary::{RelationName, TagName},
};

use super::{
    ArbitrationResolution, EventPayload, EventSource, Initiation, MergeProposalSource, ProducedBy,
    PromptTemplateName, TerminalCause, TurnRole, Volatility,
};

impl EventPayload {
    /// Typed constructors so a call site builds an event by value rather than by struct literal. The
    /// name-bearing ones accept `impl Into<MemoryName>` (so a `NamespacedMemoryName` or a `MemoryName`
    /// passes directly), and the string-bearing ones `impl Into<String>`/`impl Into<SmolStr>` (so a
    /// `&str` or an owned string passes without a manual conversion). The wide variants (five or more
    /// fields) keep their struct literals, where named fields read clearer than a long argument list.
    pub fn genesis_completed(
        manifest_hash: impl Into<String>,
        template_versions: BTreeMap<String, u32>,
    ) -> EventPayload {
        EventPayload::GenesisCompleted {
            manifest_hash: manifest_hash.into(),
            template_versions,
        }
    }

    pub fn memory_created(id: MemoryId, name: impl Into<MemoryName>) -> EventPayload {
        EventPayload::MemoryCreated {
            id,
            name: name.into(),
        }
    }

    pub fn memory_renamed(
        id: MemoryId,
        old_name: impl Into<MemoryName>,
        new_name: impl Into<MemoryName>,
    ) -> EventPayload {
        EventPayload::MemoryRenamed {
            id,
            old_name: old_name.into(),
            new_name: new_name.into(),
        }
    }

    pub fn memory_deleted(id: MemoryId) -> EventPayload {
        EventPayload::MemoryDeleted { id }
    }

    pub fn memory_superseded(id: MemoryId, entry: EntryId, superseded_by: EntryId) -> EventPayload {
        EventPayload::MemorySuperseded {
            id,
            entry,
            superseded_by,
        }
    }

    pub fn entry_temporal_resolved(
        id: MemoryId,
        entry_id: EntryId,
        occurred_at: TemporalRef,
        produced_by: Option<ProducedBy>,
    ) -> EventPayload {
        EventPayload::EntryTemporalResolved {
            id,
            entry_id,
            occurred_at,
            produced_by,
        }
    }

    pub fn entry_description_mirrored(id: MemoryId, entry_id: EntryId) -> EventPayload {
        EventPayload::EntryDescriptionMirrored { id, entry_id }
    }

    pub fn entry_temporal_resolve_failed(
        id: MemoryId,
        entry_id: EntryId,
        raw: String,
        reason: String,
        produced_by: Option<ProducedBy>,
    ) -> EventPayload {
        EventPayload::EntryTemporalResolveFailed {
            id,
            entry_id,
            raw,
            reason,
            produced_by,
        }
    }

    pub fn scheduled_job_fired(
        entry_id: EntryId,
        memory: MemoryId,
        fired_at: Timestamp,
    ) -> EventPayload {
        EventPayload::ScheduledJobFired {
            entry_id,
            memory,
            fired_at,
        }
    }

    pub fn scheduled_item_surfaced(
        entry_id: EntryId,
        memory: MemoryId,
        session: SessionId,
        surfaced_at: Timestamp,
    ) -> EventPayload {
        EventPayload::ScheduledItemSurfaced {
            entry_id,
            memory,
            session,
            surfaced_at,
        }
    }

    pub fn memory_description_regenerated(
        id: MemoryId,
        new_text: impl Into<String>,
        produced_by: Option<ProducedBy>,
    ) -> EventPayload {
        EventPayload::MemoryDescriptionRegenerated {
            id,
            new_text: new_text.into(),
            produced_by,
        }
    }

    pub fn belief_arbitrated(
        memory: MemoryId,
        competing_entries: Vec<EntryId>,
        resolution: ArbitrationResolution,
        produced_by: Option<ProducedBy>,
    ) -> EventPayload {
        EventPayload::BeliefArbitrated {
            memory,
            competing_entries,
            resolution,
            produced_by,
        }
    }

    pub fn merge_proposed(
        from: MemoryId,
        to: MemoryId,
        source: MergeProposalSource,
        rationale: Option<String>,
    ) -> EventPayload {
        EventPayload::MergeProposed {
            from,
            to,
            source,
            rationale,
        }
    }

    pub fn memory_volatility_set(id: MemoryId, volatility: Volatility) -> EventPayload {
        EventPayload::MemoryVolatilitySet { id, volatility }
    }

    pub fn describe_pass_completed(memories: Vec<MemoryId>) -> EventPayload {
        EventPayload::DescribePassCompleted { memories }
    }

    pub fn tag_created(name: TagName, description: impl Into<String>) -> EventPayload {
        EventPayload::TagCreated {
            name,
            description: description.into(),
        }
    }

    pub fn tag_description_changed(
        name: TagName,
        new_description: impl Into<String>,
    ) -> EventPayload {
        EventPayload::TagDescriptionChanged {
            name,
            new_description: new_description.into(),
        }
    }

    pub fn tag_applied_to_memory(memory: MemoryId, tag: TagName) -> EventPayload {
        EventPayload::TagAppliedToMemory { memory, tag }
    }

    pub fn tag_removed_from_memory(memory: MemoryId, tag: TagName) -> EventPayload {
        EventPayload::TagRemovedFromMemory { memory, tag }
    }

    pub fn link_removed(from: MemoryId, to: MemoryId, relation: RelationName) -> EventPayload {
        EventPayload::LinkRemoved { from, to, relation }
    }

    pub fn prompt_template_registered(
        name: PromptTemplateName,
        version: u32,
        body: impl Into<String>,
        source: EventSource,
    ) -> EventPayload {
        EventPayload::PromptTemplateRegistered {
            name,
            version,
            body: body.into(),
            source,
        }
    }

    pub fn config_set(settings: Settings, source: EventSource) -> EventPayload {
        EventPayload::ConfigSet { settings, source }
    }

    pub fn embedding_model_changed(
        from: impl Into<SmolStr>,
        to: impl Into<SmolStr>,
    ) -> EventPayload {
        EventPayload::EmbeddingModelChanged {
            from: from.into(),
            to: to.into(),
        }
    }

    pub fn conversation_started(
        id: ConversationId,
        locator: ConversationLocator,
        context_memory: MemoryId,
    ) -> EventPayload {
        EventPayload::ConversationStarted {
            id,
            locator,
            context_memory,
        }
    }

    pub fn conversation_ended(id: ConversationId) -> EventPayload {
        EventPayload::ConversationEnded { id }
    }

    pub fn session_started(
        conversation: ConversationId,
        id: SessionId,
        participants: Vec<MemoryId>,
        started_at: Timestamp,
        seeded_from_turn: Option<TurnId>,
        brief: impl Into<String>,
    ) -> EventPayload {
        EventPayload::SessionStarted {
            conversation,
            id,
            participants,
            started_at,
            seeded_from_turn,
            brief: brief.into(),
        }
    }

    pub fn conversation_turn(
        conversation: ConversationId,
        turn_id: TurnId,
        role: TurnRole,
        text: impl Into<String>,
        participant: Option<MemoryId>,
        initiation: Initiation,
        produced_by: Option<ProducedBy>,
    ) -> EventPayload {
        EventPayload::ConversationTurn {
            conversation,
            turn_id,
            role,
            text: text.into(),
            participant,
            initiation,
            produced_by,
            brief: None,
        }
    }

    pub fn lua_executed(
        conversation: ConversationId,
        turn_id: TurnId,
        script: impl Into<String>,
        result: Option<String>,
        touched: Vec<MemoryId>,
        terminal_cause: Option<TerminalCause>,
        duration_ms: u64,
    ) -> EventPayload {
        EventPayload::LuaExecuted {
            conversation,
            turn_id,
            script: script.into(),
            result,
            touched,
            terminal_cause,
            duration_ms,
        }
    }

    pub fn session_ended(conversation: ConversationId, id: SessionId) -> EventPayload {
        EventPayload::SessionEnded { conversation, id }
    }

    pub fn participant_joined(
        conversation: ConversationId,
        session: SessionId,
        participant: MemoryId,
        at_turn: TurnId,
    ) -> EventPayload {
        EventPayload::ParticipantJoined {
            conversation,
            session,
            participant,
            at_turn,
        }
    }

    pub fn participant_identified(
        memory: MemoryId,
        platform: impl Into<SmolStr>,
        platform_user_id: impl Into<SmolStr>,
    ) -> EventPayload {
        EventPayload::ParticipantIdentified {
            memory,
            platform: platform.into(),
            platform_user_id: platform_user_id.into(),
        }
    }
}
