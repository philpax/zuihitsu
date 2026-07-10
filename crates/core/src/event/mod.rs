//! The event envelope and the (deliberately growing) catalogue of event payloads.
//!
//! All state is events; graph state is a pure projection (see `docs/events-and-storage.md`). The
//! serialized payload carries only a `type` tag; field evolution rides on `#[serde(default)]` and
//! `#[serde(alias)]`, so a new capability adds a new variant or a defaulted field, and old logs
//! replay unchanged — extensibility without migrations. The derived [`EventPayload::version`]
//! stamps each type's current schema version onto the log's `version` column as recorded metadata,
//! and the materializer dispatches on the payload variant, absorbing version differences via the
//! serde defaults.

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::{
    ids::{ConversationId, EntryId, MemoryId, Seq, TurnId},
    model::{Message, ToolChoice, ToolSpec},
    prompt::PromptSectionSpan,
    time::Timestamp,
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
        /// Byte spans of `system`'s typed sections, in emission order (spec §Observability). Empty for
        /// records written before the sections were captured, so an older log replays with no spans and
        /// the console falls back to deriving the boundaries itself.
        #[serde(default)]
        system_sections: Vec<PromptSectionSpan>,
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

/// A reference to a location in a conversation — either a specific turn or the
/// conversation (room) itself. Used for attribution (`told_in`), session carryover
/// (`seeded_from_turn`), and participant joins (`at_turn`), so the frontend can uniformly
/// render every conversation reference as a cross-linkable chip. Carrying the
/// `ConversationId` lets the frontend navigate to the right room without searching.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ConversationRef {
    /// The conversation the reference belongs to.
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub conversation: ConversationId,
    /// The specific turn, or `None` for the room itself.
    pub turn: Option<TurnId>,
}

/// How widely a content entry may be surfaced (spec §Visibility). The read-time predicate
/// `visible(...)` interprets these against the present set; `PrivateToTeller` additionally never
/// surfaces to the subject of a person memory. The default is `Public`, so pre-visibility logs
/// replay with all content public — the behavior before the visibility system existed.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum Visibility {
    /// Surfaces to any present set, including the subject. Distilled into the description.
    #[default]
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
mod accessors;
mod constructors;
mod payload;

pub use payload::EventPayload;

#[cfg(test)]
mod tests;
