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
    brief::Brief,
    ids::{
        ConversationId, ConversationLocator, EntryId, MemoryId, MemoryName, Seq, SessionId, TurnId,
    },
    model::{Completion, Message, ToolChoice, ToolSpec, Usage},
    settings::Settings,
    time::{TemporalRef, Timestamp},
    vocabulary::{RelationName, TagName},
};

/// How sharply a memory's facts decay in search ranking (spec §Data model). Defaults to `Medium`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
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
}

impl std::str::FromStr for Volatility {
    type Err = ();

    /// Parse case-insensitively: the stored form is capitalized (`"Low"`/`"Medium"`/`"High"`), but the
    /// agent-facing Lua API and model replies may emit either casing.
    fn from_str(text: &str) -> Result<Volatility, Self::Err> {
        let text = text.trim();
        if text.eq_ignore_ascii_case("low") {
            Ok(Volatility::Low)
        } else if text.eq_ignore_ascii_case("medium") {
            Ok(Volatility::Medium)
        } else if text.eq_ignore_ascii_case("high") {
            Ok(Volatility::High)
        } else {
            Err(())
        }
    }
}

impl std::fmt::Display for Volatility {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str_lowercase())
    }
}

impl Volatility {
    /// The lowercase label the agent-facing API speaks (`"low"`/`"medium"`/`"high"`), distinct from
    /// the wire form [`Self::as_str`] (capitalized).
    pub fn as_str_lowercase(self) -> &'static str {
        match self {
            Volatility::Low => "low",
            Volatility::Medium => "medium",
            Volatility::High => "high",
        }
    }
}

/// A relation endpoint's cardinality. `One` means a memory has at most one link of this relation
/// in that direction (enforcement of the replace-on-`One` rule is the Lua layer's, Stage 4).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
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
}

impl std::str::FromStr for Cardinality {
    type Err = ();

    /// Parse case-insensitively: the graph layer's stored form is capitalized (`"One"`/`"Many"`),
    /// but the agent-facing Lua API speaks lowercase (`"one"`/`"many"`) and a model's reply may emit
    /// either casing.
    fn from_str(text: &str) -> Result<Cardinality, Self::Err> {
        let text = text.trim();
        if text.eq_ignore_ascii_case("one") {
            Ok(Cardinality::One)
        } else if text.eq_ignore_ascii_case("many") {
            Ok(Cardinality::Many)
        } else {
            Err(())
        }
    }
}

/// Who authored a link: the agent itself, an operator acting through the console, the
/// merge-adjudication pass that accepted an agent's cross-platform proposal on the evidence, or the
/// off-hot-path link-inference pass that extracted a relationship implicit in memory content.
/// `Adjudicated` is the one path past the operator-only merge gate, distinguishable in the log so an
/// audit can tell a console merge from an adjudicated one (spec §Cross-platform identity); `Inferred`
/// marks links the background pass authored without a teller behind them (spec §Write path → link
/// inference).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum LinkSource {
    Agent,
    Operator,
    Adjudicated,
    /// A link the off-hot-path link-inference pass authored from a relationship implicit in memory
    /// content (spec §Write path → link inference).
    Inferred,
}

impl LinkSource {
    pub fn as_str(self) -> &'static str {
        match self {
            LinkSource::Agent => "Agent",
            LinkSource::Operator => "Operator",
            LinkSource::Adjudicated => "Adjudicated",
            LinkSource::Inferred => "Inferred",
        }
    }

    /// The lowercase provenance label, matching the entry teller register: `agent` for the agent's
    /// own link, `operator` for one asserted from the console, `adjudicated` for a merge-pass
    /// `same_as`, `inferred` for one the link-inference pass authored from content. The wire/audit
    /// form is [`Self::as_str`] (capitalized); this is the agent-facing Lua label.
    pub fn as_str_lowercase(self) -> &'static str {
        match self {
            LinkSource::Agent => "agent",
            LinkSource::Operator => "operator",
            LinkSource::Adjudicated => "adjudicated",
            LinkSource::Inferred => "inferred",
        }
    }
}

impl std::str::FromStr for LinkSource {
    type Err = ();

    /// Parse case-insensitively: the stored form is capitalized (`"Agent"`/`"Operator"`/…), but the
    /// agent-facing Lua label and model replies may emit either casing.
    fn from_str(text: &str) -> Result<LinkSource, Self::Err> {
        let text = text.trim();
        if text.eq_ignore_ascii_case("agent") {
            Ok(LinkSource::Agent)
        } else if text.eq_ignore_ascii_case("operator") {
            Ok(LinkSource::Operator)
        } else if text.eq_ignore_ascii_case("adjudicated") {
            Ok(LinkSource::Adjudicated)
        } else if text.eq_ignore_ascii_case("inferred") {
            Ok(LinkSource::Inferred)
        } else {
            Err(())
        }
    }
}

/// Who raised a `MergeProposed` — the provenance the adjudicator and operator read to weigh it (spec
/// §Cross-platform identity). `Agent` is the agent's own judgment from a turn (`mem:propose_merge`);
/// `Orchestration` is the identity-resolution layer flagging that a platform arrival's handle matches
/// an existing but platform-unbound [`Namespace::Person`] stub (an agent-authored hearsay memory). An
/// orchestration proposal is never an assertion of identity — only a flag that the two may be one, for
/// the adjudicator or operator to weigh, so a handle match never itself merges two stubs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum MergeProposalSource {
    /// The default stands in for the field's absence in version-1 `MergeProposed` payloads, which
    /// predate orchestration proposals — every proposal then was the agent's own.
    #[default]
    Agent,
    Orchestration,
}

impl MergeProposalSource {
    pub fn as_str(self) -> &'static str {
        match self {
            MergeProposalSource::Agent => "Agent",
            MergeProposalSource::Orchestration => "Orchestration",
        }
    }
}

/// Provenance for events that carry an authority, distinct from a participant teller: `Bootstrap`
/// for genesis, `Orchestration` for prompt templates, `Operator` for operator/control writes, and
/// `Agent` for the agent's own (spec §Initialization, §Trust model).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum EventSource {
    Bootstrap,
    Agent,
    /// `Debugger` was this variant's name before the operator interface was renamed to the console;
    /// the alias keeps logs written under the old name readable (spec §Schema evolution: old events
    /// stay readable forever).
    #[serde(alias = "Debugger")]
    Operator,
    Orchestration,
}

/// The author of a conversation turn (spec §Event sourcing → ConversationTurn). The participant and
/// session bindings arrive with the conversation machinery at Stage 8.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
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
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum Initiation {
    Responding,
    Initiated,
}

/// How a Lua block ended when the agent saw the outcome (spec §Event sourcing). A block that
/// commits normally has no terminal cause; one the agent observed failing or deliberately aborting
/// records why.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
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
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum PromptTemplateName {
    /// The system-prompt scaffold.
    Scaffold,
    /// Synthesizes a memory's description from its entries.
    DescriptionRegen,
    /// Extracts temporal references from text.
    TemporalExtraction,
    /// Frames the pre-compaction flush turn: write durable working state to memory before the cut.
    Flush,
    /// Frames the console imprint interview: meet the creator and form self-knowledge.
    Imprint,
    /// Adjudicates a proposed cross-platform merge: weigh the two stubs' independently-recorded facts
    /// against the confidences at risk and accept or refuse.
    MergeAdjudication,
    /// Extracts relationships implicit in a memory's content and asserts them as links. The
    /// off-hot-path link-inference catch-up is gated on this template's presence: no template
    /// registered, no pass (spec §Write path → link inference).
    LinkInference,
}

impl PromptTemplateName {
    pub fn as_str(self) -> &'static str {
        match self {
            PromptTemplateName::Scaffold => "scaffold",
            PromptTemplateName::DescriptionRegen => "description-regen",
            PromptTemplateName::TemporalExtraction => "temporal-extraction",
            PromptTemplateName::Flush => "flush",
            PromptTemplateName::Imprint => "imprint",
            PromptTemplateName::MergeAdjudication => "merge-adjudication",
            PromptTemplateName::LinkInference => "link-inference",
        }
    }
}

/// Provenance for an event produced by model inference: the model and the prompt template (by name
/// and version) that wrote it (spec §Storage → provenance on inference). Carried by inference events
/// so "which model and template produced this" is answerable, and so regenerative replay knows what
/// to re-run; purely mechanical events leave it `None`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ProducedBy {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub model_id: SmolStr,
    pub template_name: PromptTemplateName,
    pub template_version: u32,
}

/// How a [`EventPayload::BeliefArbitrated`] was resolved: which competing entries the agent credited
/// (by `EntryId`) and the one-line reconciling statement it wrote (spec §Write path → arbitration).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ArbitrationResolution {
    pub credited: Vec<EntryId>,
    pub statement: String,
}

/// A relation the link-inference pass coins for a relationship no registered relation fits, recorded
/// on the `LinksInferred` audit event so the model's reasoning is replayable. Mirrors the agent
/// crate's `NewRelationSpec`, defined here so the core event type does not depend on the agent crate.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct InferredRelationSpec {
    pub name: String,
    pub inverse: String,
    pub from_card: String,
    pub to_card: String,
    pub symmetric: bool,
    pub reflexive: bool,
    #[serde(default)]
    pub description: String,
}

/// A relationship the link-inference pass identified, recorded on the `LinksInferred` audit event.
/// Mirrors the agent crate's `InferredLink`, defined here so the core event type does not depend on
/// the agent crate.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct InferredLinkSpec {
    /// The statement number (1-based) the model cited as grounding this relationship.
    pub entry: usize,
    pub relation: String,
    pub target: String,
    /// "to" (subject → target) or "from" (target → subject).
    pub direction: String,
}

/// The link-inference pass's parsed result for one memory: the new relations it proposed and the
/// links it identified, carried on the `LinksInferred` event so the model's deliberation is visible
/// in the log (spec §Write path → link inference). An empty result is recorded too — a pass that
/// found no relationships is as diagnostic as one that found the wrong ones.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct LinkInferenceResult {
    pub new_relations: Vec<InferredRelationSpec>,
    pub links: Vec<InferredLinkSpec>,
}

/// Which model call within a turn a [`EventPayload::ModelCalled`] records: a step of the agent loop,
/// or the post-turn description synthesis (spec §Observability). Paired with `turn_id`, it groups a
/// phase's calls so the prompt can be reconstructed from the [`RequestRecord`] deltas.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum ModelPhase {
    /// A step of the agent loop (the model chooses a tool call or a reply).
    Step,
    /// The post-turn description synthesis (a forced, off-buffer extraction).
    Synthesis,
}

/// The request side of a model-interaction record, stored as a delta so the agent loop's growing
/// message buffer is not repeated in full on every step (spec §Observability). A turn's phase sends
/// a frozen request shape (`system`, `tools`, `tool_choice`, `thinking`) over an append-only message
/// buffer, so the first call records a [`RequestRecord::Base`] and each later call records only the
/// messages appended since the previous one. The full prompt for any call is reconstructed by taking
/// the `Base` of its `(turn_id, phase)` group, then concatenating the `Continuation` deltas in `seq`
/// order; the frozen fields come from the `Base`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum RequestRecord {
    /// The first call of a `(turn_id, phase)` group: the frozen request shape plus the initial
    /// message buffer the call was sent.
    Base {
        system: String,
        messages: Vec<Message>,
        tools: Vec<ToolSpec>,
        tool_choice: ToolChoice,
        thinking: Option<bool>,
    },
    /// A later call in the same group: only the messages appended since the previous call.
    Continuation { appended_messages: Vec<Message> },
}

/// Who told the agent a piece of content (spec §Visibility). Distinct from [`EventSource`], which is
/// authorship *authority*: `told_by` is the *teller* whose confidence the read-time predicate
/// reasons about.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum Teller {
    /// A conversation participant, identified by their [`Namespace::Person`] memory.
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
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum Visibility {
    /// Surfaces to any present set, including the subject. Distilled into the description.
    Public,
    /// An ordinary fact learned secondhand: surfaces to any present set like `Public`, but is never
    /// distilled into a description and reaches the agent carrying a provenance marker built from the
    /// entry's `told_by`, so it always reads as "via <teller>" and the agent judges disclosure. The
    /// posture an ordinary relayed fact is classified up into, distinct from a confidence.
    Attributed,
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
    /// the field (`#[serde(default)]`).
    LinkCreated {
        from: MemoryId,
        to: MemoryId,
        #[cfg_attr(feature = "ts", ts(type = "string"))]
        relation: RelationName,
        source: LinkSource,
        #[serde(default)]
        told_by: Option<Teller>,
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

/// A committed event: a payload assigned a position in the log's total order and stamped with the
/// wall-clock time it was recorded. This is what a read returns; it is immutable. Serializable so it
/// rides verbatim over the observability surfaces (spec §Observability).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Event {
    pub seq: Seq,
    pub recorded_at: Timestamp,
    pub payload: EventPayload,
}

#[cfg(test)]
mod tests;
