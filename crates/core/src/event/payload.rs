use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::{
    brief::Brief,
    ids::{ConversationId, ConversationLocator, EntryId, MemoryId, MemoryName, SessionId, TurnId},
    model::{Completion, Usage},
    settings::Settings,
    time::{TemporalRef, Timestamp},
    vocabulary::{RelationName, TagName},
};

use super::{
    ArbitrationResolution, Cardinality, EventSource, Initiation, LinkInferenceResult, LinkSource,
    MergeProposalSource, ModelPhase, ProducedBy, PromptTemplateName, RequestRecord, Teller,
    TerminalCause, TurnRole, Visibility, Volatility,
};

/// The data carried by an event, tagged by `type` on the wire. `Seq` and `recorded_at` live on the
/// [`Event`] envelope rather than here, because they are assigned by the store at append time.
///
/// Not `Eq`: [`Settings`] carries `f32` search weights. Equality is `PartialEq` throughout.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum EventPayload {
    /// Marks a completed genesis sequence; boot branches on its presence, not on log emptiness.
    GenesisCompleted {
        manifest_hash: String,
        #[cfg_attr(feature = "ts", ts(type = "Record<string, number>"))]
        template_versions: BTreeMap<String, u32>,
    },
    /// Creates an empty memory. Initial content is recorded as a paired content-append event, so
    /// there is exactly one provenance path for all content.
    MemoryCreated { id: MemoryId, name: MemoryName },
    MemoryRenamed {
        id: MemoryId,
        old_name: MemoryName,
        new_name: MemoryName,
    },
    /// Soft delete: contents are preserved for replay and audit; the projection sets a flag.
    MemoryDeleted { id: MemoryId },
    /// Records a content entry. `told_by` is the teller, `told_in` the context it was told in (a
    /// [`Namespace::Context`] memory, resolved to its confidentiality at Stage 8; `None` until
    /// contexts exist), and `visibility` governs the read-time predicate. `asserted_at` is when
    /// the agent recorded the
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
    /// Marks an entry superseded by a newer one: the agent corrected or retracted a fact, recording
    /// which entry replaces it (spec §Visibility → superseded entries are not live, §Data model →
    /// `superseded_by`). The original `MemoryContentAppended` stays immutable; applying this stamps
    /// the superseded entry's `superseded_by`. Live surfaces then exclude it, while history surfaces
    /// (`mem:history()`, the console) still show it. `entry` and `superseded_by` belong to the same
    /// `same_as` class as `id`.
    MemorySuperseded {
        id: MemoryId,
        entry: EntryId,
        superseded_by: EntryId,
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
    /// Records that the turn-end extraction pass could not parse the model's date string for this
    /// entry. Log-only: the appended entry remains untimed, and this event surfaces the failure for
    /// operator review. `raw` is the JSON form of the extracted value for debugging.
    EntryTemporalResolveFailed {
        id: MemoryId,
        entry_id: EntryId,
        raw: String,
        reason: String,
        produced_by: Option<ProducedBy>,
    },
    /// Marks a content entry as a mirror of its memory's description — the seed entry `memory.create`
    /// appends from its `description` argument — rather than an account of a real occurrence. Applying
    /// it stamps the entry's `description_mirror` flag; the turn-end temporal extraction then skips it
    /// (see [`crate::graph::Graph::untimed_entries_since`]). A description mirror restates what the
    /// memory *is*, naming no time, so an extractor asked to time it would fabricate the conversation's
    /// "now" and that guessed date would then collide with a later, correctly-dated append on the same
    /// memory. The mark is emitted right after the seeding [`EventPayload::MemoryContentAppended`], in
    /// the same block, so the seed entry's intent survives replay rather than being re-inferred from
    /// event adjacency (which cannot tell a seeded create from a bare create followed by an append).
    EntryDescriptionMirrored { id: MemoryId, entry_id: EntryId },
    /// Fires a calendared entry's wake-up: its occurrence has come due — its `occurred_sort` passed
    /// `now`, having been later than its `asserted_at`, so it was scheduled for the future rather than
    /// recorded after the fact (spec §Scheduled work). Recorded in the log so the wake-up surface is a
    /// function of the log, not a live clock; applying it stamps the entry's `fired_at`. The fired
    /// entry waits in the surface until an eligible session drains it.
    ScheduledJobFired {
        entry_id: EntryId,
        memory: MemoryId,
        fired_at: Timestamp,
    },
    /// Marks a fired wake-up delivered: the drain raised it as an `Initiated` system turn in
    /// `session`, so it is never raised again (spec §Agent-initiated speech). Applying it stamps the
    /// entry's `surfaced_at`.
    ScheduledItemSurfaced {
        entry_id: EntryId,
        memory: MemoryId,
        session: SessionId,
        surfaced_at: Timestamp,
    },
    /// Replaces a memory's synthesized description. The text is produced by the model (Stage 5);
    /// applying it to the projection is purely mechanical. `produced_by` records the inference that
    /// wrote it (`None` only for a hand-seeded description).
    MemoryDescriptionRegenerated {
        id: MemoryId,
        new_text: String,
        produced_by: Option<ProducedBy>,
    },
    /// Records that the turn-end regeneration found conflicting statements among a memory's entries and
    /// arbitrated between them (spec §Write path → coalesce, then regenerate once). `competing_entries`
    /// is the set of conflicting entries the pass saw; `resolution` is which it credited and the
    /// reconciling note it wrote. The reconciling `resolution` stays a log-only audit record — it makes
    /// "why does the agent believe X" replayable rather than buried in a description string — but an
    /// *unresolved* arbitration (crediting neither side) projects its competing entries into the graph,
    /// so a later read renders them `disputed` (see [`crate::graph`] apply, spec §Write path → arbitration).
    BeliefArbitrated {
        memory: MemoryId,
        competing_entries: Vec<EntryId>,
        resolution: ArbitrationResolution,
        produced_by: Option<ProducedBy>,
    },
    /// A judgment that two [`Namespace::Person`] stubs may be the same human across platforms,
    /// recorded for the off-hot-path adjudication pass to weigh (spec §Cross-platform identity →
    /// adjudicated merge).
    /// `source` records who raised it: the agent from a turn (`mem:propose_merge`), or the
    /// identity-resolution orchestration when a platform arrival's handle matched an existing but
    /// platform-unbound stub. `rationale` carries the proposer's stated grounds for the adjudicator to
    /// weigh against the evidence, when the agent gave any. Deliberately *not* a `same_as` link and not
    /// projected into the graph: a proposal is inert — it leaves both stubs in their own classes and
    /// surfaces nothing — so nothing
    /// crosses the would-be merge until an adjudication accepts it, and an orchestration proposal in
    /// particular never asserts identity from a bare handle match.
    MergeProposed {
        from: MemoryId,
        to: MemoryId,
        /// Defaulted so version-1 payloads (written before the field existed) replay as
        /// agent-sourced, which every proposal then was.
        #[serde(default)]
        source: MergeProposalSource,
        /// The proposer's stated grounds for the match, if any — the coincidence the agent reasoned
        /// from (a shared wedding, the same volcanology trip). The adjudication pass reads it as the
        /// proposer's *claim*, not as evidence, weighing it against the two stubs' independently-recorded
        /// facts rather than rubber-stamping it. `None` for a proposal with no stated grounds — an
        /// orchestration handle match, or a `same_as`-via-link routed here — and defaulted so
        /// version-2 payloads (written before the field existed) replay without one.
        #[serde(default)]
        rationale: Option<String>,
    },
    /// The adjudication pass's verdict on a `MergeProposed`: whether the two stubs' independently-
    /// recorded facts coincide improbably enough to be one person, given the confidences at risk (spec
    /// §Cross-platform identity → adjudicated merge). A log-only audit record carrying the reasoning. On
    /// `accepted`, the pass also authors the `same_as` link (`LinkSource::Adjudicated`) that actually
    /// merges; on refusal, the proposal stands recorded for the operator backstop. `produced_by` records
    /// the inference, so a refusal is replayable and a wrong accept is auditable.
    MergeAdjudicated {
        from: MemoryId,
        to: MemoryId,
        accepted: bool,
        rationale: String,
        produced_by: Option<ProducedBy>,
    },
    /// The link-inference pass's parsed result for one memory (spec §Write path → link inference):
    /// the new relations the model proposed and the links it identified, whether or not any were
    /// committed (a pass that found no relationships is as diagnostic as one that found the wrong
    /// ones). Log-only audit record — the materializer ignores it, since the actual links and
    /// registrations are committed as separate `LinkCreated` / `LinkTypeRegistered` events. The
    /// pass's model-call deliberation (the prompt and completion) is recorded by the `ModelCalled`
    /// events the pass emits at `CaptureLevel::Full`; this event carries the structured outcome so
    /// the log shows what the model decided without reconstructing it from the raw completion.
    LinksInferred {
        memory: MemoryId,
        result: LinkInferenceResult,
        produced_by: Option<ProducedBy>,
    },
    /// Records one describer pass: every memory the pass considered, whether or not synthesis
    /// succeeded (matching the describer's advance-past-failure discipline). Applying it stamps each
    /// listed memory's `last_described_seq` to this event's seq, so a memory is stale exactly while
    /// its `last_content_seq` outruns its `last_described_seq` (spec §Write path → regenerate off the
    /// hot path, as a catch-up). The list is a `Vec` so a pass may batch several memories, though a
    /// per-memory pass records a batch of one. Log-derived state: the describe backlog survives a
    /// restart, since it is a function of the log rather than an in-memory cursor.
    DescribePassCompleted { memories: Vec<MemoryId> },
    MemoryVolatilitySet {
        id: MemoryId,
        volatility: Volatility,
    },
    /// Creates a tag, which always forces a purpose. Distinct from application, which never mutates
    /// the description (spec §Lua API → tags).
    TagCreated {
        #[cfg_attr(feature = "ts", ts(type = "string"))]
        name: TagName,
        description: String,
    },
    TagDescriptionChanged {
        #[cfg_attr(feature = "ts", ts(type = "string"))]
        name: TagName,
        new_description: String,
    },
    TagAppliedToMemory {
        memory: MemoryId,
        #[cfg_attr(feature = "ts", ts(type = "string"))]
        tag: TagName,
    },
    TagRemovedFromMemory {
        memory: MemoryId,
        #[cfg_attr(feature = "ts", ts(type = "string"))]
        tag: TagName,
    },
    /// Registers a relation in the schema, accessible under either label; the inverse view's
    /// cardinality is computed (spec §Data model: the registry lives in data, not code). The
    /// description is the relation's one-line purpose, surfaced in the system prompt's relation
    /// registry and in `links.list`/`get` so the agent knows which relation fits which situation.
    LinkTypeRegistered {
        #[cfg_attr(feature = "ts", ts(type = "string"))]
        name: RelationName,
        #[cfg_attr(feature = "ts", ts(type = "string"))]
        inverse: RelationName,
        from_card: Cardinality,
        to_card: Cardinality,
        symmetric: bool,
        reflexive: bool,
        description: String,
    },
    /// Creates a directed edge. The materializer canonicalizes direction at write time, so a link
    /// asserted under either label produces the same stored edge. `told_by` is the teller who asserted
    /// the relationship — the provenance an asymmetric-belief relation turns on (who claims that the
    /// edge holds), carried for every link the same way an entry carries its teller. `None` for a link
    /// with no teller behind it (the adjudicated `same_as`), and for pre-provenance logs that predate
    /// the field (`#[serde(default)]`). `visibility` is the audience posture — `Public` for
    /// structural/operator/adjudicated links, `PrivateToTeller` for a participant-asserted belief about
    /// someone else, `Attributed` for a secondhand relayed relationship. Defaults to `Public` for
    /// pre-visibility logs. `told_in` carries the context memory (room) the link was asserted in,
    /// mirroring content entries' `told_in` — the provenance a teller-private marker's room reference
    /// reads. Defaults to `None` for pre-visibility logs.
    LinkCreated {
        from: MemoryId,
        to: MemoryId,
        #[cfg_attr(feature = "ts", ts(type = "string"))]
        relation: RelationName,
        source: LinkSource,
        #[serde(default)]
        told_by: Option<Teller>,
        #[serde(default)]
        told_in: Option<MemoryId>,
        #[serde(default)]
        visibility: Visibility,
    },
    LinkRemoved {
        from: MemoryId,
        to: MemoryId,
        #[cfg_attr(feature = "ts", ts(type = "string"))]
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
    /// Records an embedding-model swap: the model that produced the existing vectors (`from`) gave way
    /// to a new one (`to`). The model identity is environmental config, but *changing* it is a
    /// behaviorally-significant, logged migration, because it invalidates every stored vector — cosine
    /// across two embedding spaces is silently wrong — so it brackets a full re-embed of the log under
    /// the new model. Detected at boot and acted on there (the index is cleared and rebuilt before the
    /// server serves; see spec §Storage → vector store). The graph ignores it; it bears only on the
    /// vector index.
    EmbeddingModelChanged {
        #[cfg_attr(feature = "ts", ts(type = "string"))]
        from: SmolStr,
        #[cfg_attr(feature = "ts", ts(type = "string"))]
        to: SmolStr,
    },
    /// Records one executed Lua block — what the agent saw. The stored `result` is the value
    /// rendered back into the next inference step (text, not a live handle), so faithful replay
    /// feeds the model exactly the string it saw. `touched` is the set of memories the block read
    /// or wrote; `terminal_cause` is set only for agent-visible error/abort outcomes. `duration_ms`
    /// is the block's wall-clock execution time (the final attempt's, on the retry path), recorded
    /// for the console's turn timeline; `#[serde(default)]` so pre-timing logs replay as `0`.
    LuaExecuted {
        conversation: ConversationId,
        turn_id: TurnId,
        script: String,
        result: Option<String>,
        touched: Vec<MemoryId>,
        terminal_cause: Option<TerminalCause>,
        #[serde(default)]
        duration_ms: u64,
    },
    /// Records one model call's interaction — the deliberation surface the console reconstructs
    /// (spec §Observability). Log-only telemetry: the materializer ignores it, so faithful replay's
    /// rebuilt state is identical with or without it, and the recorded (non-deterministic) reasoning,
    /// usage, and latency are reproduced verbatim because replay reads them rather than recomputing.
    /// `request` is `Some` at the `Full` capture level (the delta-encoded [`RequestRecord`]) and
    /// `None` at `Digest`; `request_digest` is a `sha2::Sha256` over the full request actually sent,
    /// always present, so a reconstructed prompt can be checked against it.
    ModelCalled {
        conversation: ConversationId,
        turn_id: TurnId,
        phase: ModelPhase,
        request_digest: String,
        request: Option<RequestRecord>,
        completion: Completion,
        reasoning: Option<String>,
        finish_reason: Option<String>,
        usage: Usage,
        duration_ms: u64,
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
        /// The structured brief behind a mid-session join's `system` turn (spec §Mid-conversation
        /// joins): the same content `text` carries as rendered markup, kept as data so a structured
        /// consumer (the console) renders it as a proper entrance treatment rather than surfacing the
        /// raw markup. `None` for every other turn — an inbound message, the agent's reply, a wake-up
        /// surface — and defaulted so version-1 payloads (written before the field existed) replay
        /// without one.
        #[serde(default)]
        brief: Option<Brief>,
    },
    /// Opens a durable conversation (a room), keyed by its `locator`. Fires once on first contact;
    /// the room then persists across sessions for the agent's life (spec §Conversations).
    /// `context_memory` is the [`Namespace::Context`] memory minted eagerly alongside the room, so
    /// the locator resolves to a first-class memory the agent can tag (`#confidential`) and reason
    /// about.
    ConversationStarted {
        id: ConversationId,
        locator: ConversationLocator,
        context_memory: MemoryId,
    },
    /// Retires a conversation permanently — rare, since conversations are durable.
    ConversationEnded { id: ConversationId },
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
    /// Binds a [`Namespace::Person`] stub to a platform identity, seeding the `(platform,
    /// platform_user_id) -> memory_id` operational mapping (spec §Identity). Emitted on first
    /// contact (with the
    /// `MemoryCreated` that mints the stub) and whenever an existing stub gains a further platform
    /// identity. The mapping is operational, not a memory-graph fact, so it lives in this event
    /// rather than as a relation.
    ParticipantIdentified {
        memory: MemoryId,
        #[cfg_attr(feature = "ts", ts(type = "string"))]
        platform: SmolStr,
        #[cfg_attr(feature = "ts", ts(type = "string"))]
        platform_user_id: SmolStr,
    },
}
