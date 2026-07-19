//! The composed query DTOs the [`Replica`](crate::Replica) returns across the wasm boundary.
//!
//! Each DTO is defined once, here: the value the wasm serializes and the TypeScript declaration the
//! console consumes both derive from the one struct, so the two sides cannot drift — the `Replica`
//! methods construct and return these directly, with no hand-written TypeScript mirror and no cast
//! on the console side.
//!
//! Every field whose type is a `zuihitsu-core` view — [`MemoryView`], [`Timestamp`], the id newtypes —
//! already has a generated definition in `console/packages/wire/types/`. The [`CORE_TYPE_IMPORTS`]
//! bridge re-imports those into the generated declarations so there is exactly one definition of
//! each, shared by the DTOs here and the views that consume the same core types directly.

use serde::Serialize;
use tsify_next::Tsify;
use wasm_bindgen::prelude::*;
use zuihitsu_core::{
    event::MergeProposalSource,
    graph::{EntryView, LinkView, MemoryView},
    ids::{ConversationId, EntryId, MemoryId, MemoryName, SessionId},
    time::Timestamp,
};

/// Pull the `zuihitsu-core` view types the DTOs below reference into the generated `console_wasm.d.ts`,
/// so the DTO declarations resolve to the single definition in the sibling `types/` directory
/// rather than a second copy. Emitted verbatim into the generated declarations as a leading import
/// block.
#[wasm_bindgen(typescript_custom_section)]
const CORE_TYPE_IMPORTS: &'static str = r#"
import type { EntryId } from "../types/EntryId";
import type { EntryView } from "../types/EntryView";
import type { LinkView } from "../types/LinkView";
import type { MemoryView } from "../types/MemoryView";
import type { MemoryName } from "../types/MemoryName";
import type { MemoryId } from "../types/MemoryId";
import type { MergeProposalSource } from "../types/MergeProposalSource";
import type { Timestamp } from "../types/Timestamp";
import type { SessionId } from "../types/SessionId";
import type { ConversationId } from "../types/ConversationId";
"#;

/// Everything the State view shows when a memory is opened: the memory itself, its live content
/// entries, its full history (including superseded entries), its links, and its `same_as` class.
/// Composed from several core reads so the frontend opens a memory in one call.
#[derive(Serialize, Tsify)]
#[tsify(into_wasm_abi, missing_as_null, hashmap_as_object)]
pub struct MemoryDetail {
    pub memory: MemoryView,
    pub entries: Vec<EntryView>,
    pub history: Vec<EntryView>,
    pub links: Vec<LinkView>,
    pub class: Vec<MemoryView>,
    /// The entry ids currently under an unresolved belief arbitration, so the view can mark a contested
    /// fact as disputed (the same signal the agent sees on a read).
    pub disputed: Vec<EntryId>,
}

/// One cross-platform merge proposal as the console surfaces it (spec §Cross-platform identity): the
/// two stubs by handle *and* id (so the view can name them and deep-link into State), who raised it,
/// the proposer's stated grounds if any, and where the proposal now stands. Unlike the operator
/// backstop — which drops a settled proposal — the console keeps every proposal so it can show the
/// whole record: which pairs the agent flagged and which the operator has since confirmed.
#[derive(Serialize, Tsify)]
#[tsify(into_wasm_abi, missing_as_null, hashmap_as_object)]
pub struct MergeProposalView {
    pub from: MemoryName,
    pub to: MemoryName,
    pub from_id: MemoryId,
    pub to_id: MemoryId,
    pub source: MergeProposalSource,
    /// The proposer's stated grounds for the match — the coincidence the agent reasoned from. `None` for
    /// an orchestration handle match or a `same_as`-via-link, which carry no rationale.
    pub rationale: Option<String>,
    pub status: MergeStatus,
    /// Whether each stub is currently its `same_as` class's primary — the id class-level reads resolve
    /// through — so the view marks the canonical stub. Once merged exactly one of a pair is primary,
    /// unless a third, older member holds the class, in which case neither is.
    pub from_primary: bool,
    pub to_primary: bool,
    /// Whether the operator has pinned each stub as its class's primary (`ClassPrimaryDesignated`), as
    /// opposed to it winning by the earliest-ULID default. The view offers a pinned stub a release and
    /// an unpinned, non-primary stub a designation.
    pub from_designated: bool,
    pub to_designated: bool,
}

/// Where a merge proposal stands at the current fold horizon: still awaiting the operator's decision, or
/// merged (the operator confirmed it and the two stubs now share a `same_as` class).
#[derive(Serialize, Tsify)]
#[tsify(into_wasm_abi, missing_as_null, hashmap_as_object)]
#[serde(rename_all = "snake_case")]
pub enum MergeStatus {
    Pending,
    Merged,
}

/// One item on the agent's agenda: when it occurs, the memory it lives in, the text, and whether it
/// is a recurring instance. One-offs come from `occurrences_in_window`; recurring instances from
/// `recurring_instances_in_window`, which expands each rule through the agent's own `next_occurrence`
/// so the projection cannot drift from the agent's scheduling.
#[derive(Serialize, Tsify)]
#[tsify(into_wasm_abi, missing_as_null, hashmap_as_object)]
pub struct AgendaItem {
    pub when: Timestamp,
    /// The occurrence is a whole day or fuzzier span, not a precise instant, so the calendar renders
    /// it without a clock time (a `Day` sorts at noon — not a stated time). See `TemporalRef::is_all_day`.
    pub all_day: bool,
    pub memory: String,
    pub text: String,
    pub recurring: bool,
}

/// A durable conversation (room) with its sessions, the backbone of the Conversation view. The
/// turns themselves render off the event stream; this supplies the structure and the names the raw
/// log only carries as ids — the room's [`Namespace::Context`](zuihitsu_core::ids::Namespace) name
/// and each session's participant handles.
#[derive(Serialize, Tsify)]
#[tsify(into_wasm_abi, missing_as_null, hashmap_as_object)]
pub struct ConversationDetail {
    pub id: ConversationId,
    pub platform: String,
    pub scope_path: String,
    pub context_name: Option<String>,
    pub sessions: Vec<SessionSummary>,
}

/// One activity window within a conversation: when it opened, the brief frozen at its start, and the
/// participants present, resolved to their memory handles.
#[derive(Serialize, Tsify)]
#[tsify(into_wasm_abi, missing_as_null, hashmap_as_object)]
pub struct SessionSummary {
    pub id: SessionId,
    pub started_at: Timestamp,
    pub brief: String,
    pub participants: Vec<String>,
}

/// A resolved memory reference, crossing to the console's `MemRefChip`: the `same_as` class primary the
/// reference collapses to (the memory the chip opens) and its handle (the chip's label).
#[derive(Serialize, Tsify)]
#[tsify(into_wasm_abi, missing_as_null, hashmap_as_object)]
#[serde(rename_all = "camelCase")]
pub struct MemRefResolution {
    pub primary_id: String,
    pub handle: String,
}

/// One model call's digest verification result, keyed by the `ModelCalled` event's seq.
#[derive(Serialize, Tsify)]
#[tsify(into_wasm_abi, missing_as_null, hashmap_as_object)]
pub struct DigestCheck {
    #[tsify(type = "number")]
    pub seq: u64,
    pub status: DigestStatus,
}

/// How one model call's recorded prompt compares against the digest stamped at send time. `verified`
/// means the displayed prompt provably matches the wire request; `mismatch` means it must not be
/// trusted silently; `unverifiable` marks a structured synthesis call, whose response format is not
/// recorded; `unrecorded` marks a call whose request was not captured.
#[derive(Serialize, Clone, Copy, Tsify)]
#[tsify(into_wasm_abi, missing_as_null, hashmap_as_object)]
#[serde(rename_all = "snake_case")]
pub enum DigestStatus {
    Verified,
    Mismatch,
    Unverifiable,
    Unrecorded,
}

/// One span of the combined reference scan, crossing to the console's remark pass: literal prose, a turn
/// reference, or a memory reference, each carrying its subject's ULID. The `kind` tag is what the remark
/// pass dispatches on to mint the matching chip.
#[derive(Serialize, Tsify)]
#[tsify(into_wasm_abi, missing_as_null, hashmap_as_object)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RefSegment {
    Prose { text: String },
    Turn { id: String },
    Mem { id: String },
}

/// A JSON array of [`MergeProposalView`] as it crosses the boundary. Tsify cannot lower a bare
/// `Vec<T>` of a DTO through the wasm ABI (the element lacks the vector ABI trait), so the list rides a
/// transparent newtype: serde flattens it away, keeping the crossing a plain JSON array, and Tsify emits
/// it as `MergeProposalView[]`.
#[derive(Serialize, Tsify)]
#[tsify(into_wasm_abi, missing_as_null, hashmap_as_object)]
#[serde(transparent)]
pub struct MergeProposalList(pub Vec<MergeProposalView>);

/// A JSON array of [`AgendaItem`] as it crosses the boundary. See [`MergeProposalList`] for why the
/// list is wrapped.
#[derive(Serialize, Tsify)]
#[tsify(into_wasm_abi, missing_as_null, hashmap_as_object)]
#[serde(transparent)]
pub struct AgendaList(pub Vec<AgendaItem>);

/// A JSON array of [`ConversationDetail`] as it crosses the boundary. See [`MergeProposalList`] for why
/// the list is wrapped.
#[derive(Serialize, Tsify)]
#[tsify(into_wasm_abi, missing_as_null, hashmap_as_object)]
#[serde(transparent)]
pub struct ConversationList(pub Vec<ConversationDetail>);

/// A JSON array of [`DigestCheck`] as it crosses the boundary. See [`MergeProposalList`] for why the
/// list is wrapped.
#[derive(Serialize, Tsify)]
#[tsify(into_wasm_abi, missing_as_null, hashmap_as_object)]
#[serde(transparent)]
pub struct DigestCheckList(pub Vec<DigestCheck>);

/// A JSON array of [`RefSegment`] as it crosses the boundary. See [`MergeProposalList`] for why the
/// list is wrapped.
#[derive(Serialize, Tsify)]
#[tsify(into_wasm_abi, missing_as_null, hashmap_as_object)]
#[serde(transparent)]
pub struct RefSegmentList(pub Vec<RefSegment>);
