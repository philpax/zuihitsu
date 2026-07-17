//! Deterministic contextual-brief composition (spec §Contextual briefs).
//!
//! At session start the composer assembles the agent's hot context — the **self** brief, the
//! **current room** (with its tags), and a **per-participant** brief for each present participant —
//! into a block that is frozen into the system prompt for the session. Every fact is filtered
//! through the visibility predicate against the *full, uncapped* present set; the present-set cap
//! governs only which participants get a full block, never what the predicate sees (spec §Present-set
//! cap → invariant). The two sets are kept distinct by construction, so a high-population room can't
//! leak through the cap.
//!
//! Composition reads the graph but runs no model and makes no relevance judgment beyond a
//! deterministic recency ranking, so a leak into a brief is a mechanism bug catchable without
//! inference. The active-context and tag-vocabulary sections (spec §Composition 4–5) and rich
//! active-threads arrive with the compaction/working-set machinery; this cut composes the self,
//! current-context, and per-participant sections, which is what the brief-surface fixtures gate.

use std::{collections::HashSet, fmt::Write as _};

use serde::{Deserialize, Serialize};

use crate::{
    event::Visibility,
    graph::{Graph, GraphError},
    ids::{MemoryId, MemoryName},
    settings::BriefSettings,
    time::{self, Timestamp},
    vocabulary::{RelationName, TagName},
};

use crate::visibility::{self};

use helpers::{ranked_present, render_memory_body};

/// A failure composing the brief, delegating to the graph beneath it.
#[derive(Debug)]
pub enum BriefError {
    Graph(GraphError),
}

impl std::fmt::Display for BriefError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BriefError::Graph(error) => write!(f, "brief (graph): {error}"),
        }
    }
}

impl std::error::Error for BriefError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            BriefError::Graph(error) => Some(error),
        }
    }
}

impl From<GraphError> for BriefError {
    fn from(error: GraphError) -> BriefError {
        BriefError::Graph(error)
    }
}

/// A single participant's brief, as structured data rather than as flattened markup: the subject
/// (the memory the block is about), a prose summary, the visible recent facts each with their
/// provenance/staleness markers kept beside the text, and the key relationships. This is the source
/// of truth a mid-session join carries on its `system` turn; [`Brief::render`] is the projection that
/// produces the exact markup the agent's prompt reads, so a structured consumer (the console) sees the
/// parts without parsing them back out of the text (spec §Mid-conversation joins).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
// Deserialization routes through [`BriefWire`] so a brief recorded before a relationship named both
// endpoints — an edge stored only as its neighbour `subject` — still loads: the wire form reconstructs
// the missing `source`/`target` from this brief's own subject. Serialization stays the derived new form.
#[serde(try_from = "BriefWire")]
pub struct Brief {
    /// The memory this block is about — the `## <subject>` header of the rendered form.
    pub subject: MemoryName,
    /// The memory's prose description, absent when it has none.
    pub summary: Option<String>,
    /// The visible recent facts, oldest first, in the same recency window the composer applies.
    pub recent_facts: Vec<BriefFact>,
    /// The memory's key relationships, each `source → relation → target`.
    pub relationships: Vec<BriefRelationship>,
}

/// One recent fact in a [`Brief`]: the fact text and the provenance/staleness markers that trail it —
/// the teller-private `[via …]`/`[private · …]` attribution and the staleness note. The markers are
/// kept beside the text (rather than baked into one string) so the console can style them quietly; the
/// markup projection appends them space-separated, reproducing the flat line the agent reads.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct BriefFact {
    pub text: String,
    /// The markers appended after the fact text, in order (the visibility marker, then staleness).
    pub markers: Vec<String>,
}

/// One relationship in a [`Brief`]: the edge's two endpoints in their stored direction and the relation
/// between them, rendered as `source → relation → target [marker]`. Both endpoints are named explicitly
/// (rather than leaving this brief's own identity implicit) so the direction reads off the line — an edge
/// running *into* this identity has the neighbour as `source` and this identity as `target`, an edge
/// running out reverses it. A brief recorded before this pairing stored only the neighbour as `subject`;
/// [`BriefWire`] reconstructs the endpoints for such a log at deserialization. `relation` serializes as
/// its bare label (the wire form [`RelationName`] keeps), so it is typed as a `string` on the console
/// side. `marker` carries the provenance marker for a non-public link (`[via Erin]` or
/// `[teller-private, …]`), appended after the relationship line since a link has no text body. The
/// relationship carries no fact of its own: the neighbour's substance lives in the neighbour, reached by
/// its handle, not duplicated onto every edge that points at it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct BriefRelationship {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub relation: RelationName,
    /// The edge's `from` endpoint — this identity for an outgoing edge, the neighbour for an incoming one.
    pub source: MemoryName,
    /// The edge's `to` endpoint — the neighbour for an outgoing edge, this identity for an incoming one.
    pub target: MemoryName,
    /// The provenance marker for a non-public link, appended after the relationship line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub marker: Option<String>,
}

impl Brief {
    /// Render the brief as the `## <subject>` participant block — the agent-facing projection of
    /// the structured form, and the one place the block's markup is written.
    pub fn render(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "## {}", self.subject.as_str());
        self.render_body(&mut out);
        out
    }

    /// Render just the body — summary, recent facts, and relationships — without the `## <subject>`
    /// header, the shape [`compose`] writes under a header it has already emitted itself.
    fn render_body(&self, out: &mut String) {
        if let Some(summary) = &self.summary {
            let _ = writeln!(out, "<summary>{summary}</summary>");
        }
        if !self.recent_facts.is_empty() {
            out.push_str("<recent_facts>\n");
            for fact in &self.recent_facts {
                let _ = writeln!(out, "- {}", fact.render());
            }
            out.push_str("</recent_facts>\n");
        }
        if !self.relationships.is_empty() {
            out.push_str("<relationships>\n");
            for relationship in &self.relationships {
                let _ = write!(
                    out,
                    "- {} → {} → {}",
                    relationship.source.as_str(),
                    relationship.relation.as_str(),
                    relationship.target.as_str()
                );
                if let Some(marker) = &relationship.marker {
                    let _ = write!(out, " {marker}");
                }
                out.push('\n');
            }
            out.push_str("</relationships>\n");
        }
    }
}

impl BriefFact {
    /// The flat line the markup carries: the fact text with each marker appended, space-separated.
    fn render(&self) -> String {
        let mut line = self.text.clone();
        for marker in &self.markers {
            line.push(' ');
            line.push_str(marker);
        }
        line
    }
}

/// The on-the-wire shape [`Brief`] deserializes through, so both the current form (each relationship
/// naming its `source` and `target`) and a pre-pairing log (each relationship storing only the neighbour
/// as `subject`) load. The conversion to [`Brief`] is where the reconstruction happens: it has the
/// brief's own subject, the near end a bare `subject` row left implicit.
#[derive(Deserialize)]
struct BriefWire {
    subject: MemoryName,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    recent_facts: Vec<BriefFact>,
    #[serde(default)]
    relationships: Vec<RelationshipWire>,
}

/// One relationship as it may appear in a stored brief: the current form carries `source` and `target`;
/// a pre-pairing log carries only `subject` (the neighbour), with the direction dropped and this
/// identity implicit.
#[derive(Deserialize)]
struct RelationshipWire {
    relation: RelationName,
    #[serde(default)]
    source: Option<MemoryName>,
    #[serde(default)]
    target: Option<MemoryName>,
    #[serde(default)]
    subject: Option<MemoryName>,
    #[serde(default)]
    marker: Option<String>,
}

impl TryFrom<BriefWire> for Brief {
    type Error = String;

    fn try_from(wire: BriefWire) -> Result<Brief, String> {
        let relationships = wire
            .relationships
            .into_iter()
            .map(|relationship| {
                let (source, target) = match (relationship.source, relationship.target) {
                    (Some(source), Some(target)) => (source, target),
                    // A pre-pairing log stored only the neighbour as `subject`, with this identity the
                    // implicit near end and the edge rendered outgoing. Reconstruct that as
                    // subject → neighbour, matching how it read before the endpoints were named.
                    _ => {
                        let neighbour = relationship.subject.ok_or_else(|| {
                            "brief: relationship carries neither source/target nor subject"
                                .to_owned()
                        })?;
                        (wire.subject.clone(), neighbour)
                    }
                };
                Ok(BriefRelationship {
                    relation: relationship.relation,
                    source,
                    target,
                    marker: relationship.marker,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(Brief {
            subject: wire.subject,
            summary: wire.summary,
            recent_facts: wire.recent_facts,
            relationships,
        })
    }
}

/// The session-specific inputs to [`compose`]: who is present, the current room, the working set
/// carried across a compaction seam, and the session's start time. Bundled into a request so the call
/// reads clearly (`compose(graph, settings, &request)`) rather than as a row of bare arguments.
pub struct BriefRequest<'a> {
    /// The full present set — the visibility predicate always resolves against all of it.
    pub present_set: &'a [MemoryId],
    /// The room's [`Namespace::Context`] memory, if any.
    pub current_context: Option<MemoryId>,
    /// Memories carried across a compaction seam (empty otherwise), rendered as active threads.
    pub working_set: &'a [MemoryId],
    /// The session's start time — the reference for the `<upcoming/>` window.
    pub now: Timestamp,
}

/// Compose the contextual brief for the session described by `request`. The predicate always resolves
/// against the full present set; `settings.present_set_cap` bounds only how many participants get a
/// full block, with the remainder collapsed to name-only. The working set is re-filtered through
/// `visible` against the present set like any other block (spec §Compaction → working-set carryover).
///
/// The whole brief is bounded by `settings.char_budget`: the self and current-room blocks always
/// render, then the ranked participant, active-thread, and upcoming blocks are packed in priority
/// order until the budget is spent (see [`compose_packed`]).
pub fn compose(
    graph: &Graph,
    settings: &BriefSettings,
    request: &BriefRequest,
) -> Result<String, BriefError> {
    Ok(compose_packed(graph, settings, request)?.text)
}

/// The packed brief text plus the record of which optional blocks the char budget admitted, so the
/// composition trace ([`compose_traced`]) can mark exactly what surfaced without re-deriving the
/// packing.
pub(super) struct PackedBrief {
    pub text: String,
    pub plan: BriefPlan,
}

/// Which optional blocks the char budget admitted as full blocks. The self and current-room blocks
/// are mandatory — they always render and so are not tracked here.
pub(super) struct BriefPlan {
    /// Present participants rendered as a full block (the rest collapsed to a name-only line).
    pub full_participants: HashSet<MemoryId>,
    /// Active-thread memories the budget admitted.
    pub included_threads: HashSet<MemoryId>,
}

/// Compose the brief and pack it under the char budget. Priority is by section: the **self** and
/// **current-room** blocks are mandatory and always render (a budget can neither erase who the agent
/// is nor where it stands); then **present participants** in recency rank, each dropping to a name-only
/// line once the budget can no longer afford its full block; then **active threads**; then **upcoming**.
/// Packing is by whole block — a fact is never truncated mid-text — because the within-block flood
/// vectors are already bounded by `recent_facts` and `key_relationships`, so block granularity is what
/// remains to bound. A name-only line always renders for a present participant, since presence itself
/// drives the visibility predicate and must be shown even when the budget is exhausted.
pub(super) fn compose_packed(
    graph: &Graph,
    settings: &BriefSettings,
    request: &BriefRequest,
) -> Result<PackedBrief, BriefError> {
    let &BriefRequest {
        present_set,
        current_context,
        working_set,
        now,
    } = request;
    // The visibility predicate resolves identity over the `same_as` class.
    let class_of = |id| graph.class_id(id).map(|class| class.unwrap_or(id));
    let mut budget = Budget::new(settings.char_budget);
    let mut out = String::new();
    let mut full_participants = HashSet::new();
    let mut included_threads = HashSet::new();
    // A `same_as` class is one identity, so it earns one block: the class reads (`class_entries`,
    // `class_neighbor_links`) already resolve the whole class, so a second stub would render the same
    // facts and relationships again. This tracks the classes already rendered (self, each present
    // participant, each active thread) so a later stub of an identity already shown is skipped.
    let mut rendered_classes: HashSet<MemoryId> = HashSet::new();

    // 1. Self brief — the agent's own memory in the per-participant shape. Mandatory.
    if let Some(self_memory) = graph.self_memory()? {
        let mut block = String::from("# You\n");
        render_memory_body(
            &mut block,
            graph,
            &self_memory,
            present_set,
            &class_of,
            settings,
            now,
        )?;
        block.push('\n');
        budget.charge(char_len(&block));
        out.push_str(&block);
        rendered_classes.insert(class_of(self_memory.id)?);
    }

    // 2. Current context — the room — with its #confidential tag, visible regardless of who is
    //    present (it is a memory-level tag, not a teller-gated entry). Mandatory.
    if let Some(context_id) = current_context
        && let Some(context) = graph.memory_by_id(context_id)?
    {
        let mut block = String::new();
        let room = visibility::room_display(context.name.as_str());
        if context.tags.contains(&TagName::Confidential) {
            let _ = writeln!(block, "# Current room: {room} (confidential)");
        } else {
            let _ = writeln!(block, "# Current room: {room}");
        }
        if !context.description.is_empty() {
            let _ = writeln!(block, "{}", context.description);
        }
        block.push('\n');
        budget.charge(char_len(&block));
        out.push_str(&block);
    }

    // 3. Per-participant briefs, ranked by recency, capped, and packed. The predicate above and below
    //    always sees the full present set; the cap and the budget only govern who gets a full block.
    if !present_set.is_empty() {
        out.push_str("# Present\n");
        budget.charge(char_len("# Present\n"));
        let cap = settings.present_set_cap.max(0) as usize;
        for (index, participant) in ranked_present(graph, present_set)?.into_iter().enumerate() {
            let Some(memory) = graph.memory_by_id(participant)? else {
                continue;
            };
            // One block per identity: a present stub whose class is already shown (self, or an
            // earlier-ranked present stub of the same person) is skipped rather than repeated.
            if !rendered_classes.insert(class_of(participant)?) {
                continue;
            }
            let mut placed_full = false;
            if index < cap {
                let mut block = String::new();
                let _ = writeln!(block, "## {}", memory.name.as_str());
                render_memory_body(
                    &mut block,
                    graph,
                    &memory,
                    present_set,
                    &class_of,
                    settings,
                    now,
                )?;
                if budget.take(char_len(&block)) {
                    out.push_str(&block);
                    full_participants.insert(participant);
                    placed_full = true;
                }
            }
            if !placed_full {
                // Below the cap, or the budget can no longer afford the full block: collapse to a
                // name-only line. It still renders — presence drives the predicate (spec §Present-set
                // cap → invariant) — so its small cost is charged rather than gated.
                let line = format!("- {} (present)\n", memory.name.as_str());
                budget.charge(char_len(&line));
                out.push_str(&line);
            }
        }
    }

    // 4. Active threads — the working set the session opened with: either the memories the ending
    //    session touched, carried across a compaction seam, or, for a session that opens cold (an idle
    //    gap or first contact, with no carryover), the memories recent sessions touched, so a fresh
    //    session re-surfaces the threads a warm continuation would. Either way each is rendered in the
    //    per-participant shape, so its facts are re-filtered through `visible` against the new present
    //    set (an aside about a now-present subject is suppressed). Self, the current room, and present
    //    participants are already shown above, so they are skipped to avoid duplication. Packed per
    //    thread, under a header charged only if at least one thread is admitted.
    if !working_set.is_empty() {
        let self_id = graph.self_memory()?.map(|memory| memory.id);
        let header = "# Active threads\n";
        if budget.take(char_len(header)) {
            let mut threads = String::new();
            for &id in working_set {
                if Some(id) == self_id || Some(id) == current_context || present_set.contains(&id) {
                    continue;
                }
                // Collapse a same_as class to one thread: skip a working-set stub whose identity is
                // already shown (self, a present participant, or an earlier thread of the same person),
                // which would otherwise repeat the same class-wide facts and relationships. The class is
                // marked below only once a block actually renders, so a stub with an all-filtered body
                // does not suppress a sibling that carries a summary.
                let class = class_of(id)?;
                if rendered_classes.contains(&class) {
                    continue;
                }
                let Some(memory) = graph.memory_by_id(id)? else {
                    continue;
                };
                let mut body = String::new();
                render_memory_body(
                    &mut body,
                    graph,
                    &memory,
                    present_set,
                    &class_of,
                    settings,
                    now,
                )?;
                if body.trim().is_empty() {
                    continue;
                }
                let mut block = String::new();
                let _ = writeln!(block, "## {}", memory.name.as_str());
                block.push_str(&body);
                if budget.take(char_len(&block)) {
                    threads.push_str(&block);
                    included_threads.insert(id);
                    rendered_classes.insert(class);
                }
            }
            if threads.is_empty() {
                budget.refund(char_len(header));
            } else {
                out.push_str(header);
                out.push_str(&threads);
            }
        }
    }

    // 5. Upcoming — near-future calendared items, soonest first, so the agent organically raises
    //    them (spec §Calendar → <upcoming/>). Each occurrence is filtered through `visible` like any
    //    entry — a private aside about an absent person carries its marker, one about a now-present
    //    subject is suppressed — capped by `max_upcoming_items`, and packed per line under the budget.
    let window_days = settings.upcoming_window_days.max(0);
    let max_items = settings.max_upcoming_items.max(0) as usize;
    if window_days > 0 && max_items > 0 {
        let to = Timestamp::from_millis(
            now.as_millis()
                .saturating_add(window_days * time::MILLIS_PER_DAY),
        );
        let mut lines = Vec::new();
        for (memory, entry) in graph.occurrences_in_window(now, to)? {
            if lines.len() >= max_items {
                break;
            }
            if !visibility::visible(&entry, &memory, present_set, &class_of)? {
                continue;
            }
            let when = entry
                .occurred_sort
                .map_or_else(String::new, time::format_day);
            let mut line = format!("- {when}: {} — {}", memory.name.as_str(), entry.text);
            if entry.visibility != Visibility::Public {
                let teller = graph.teller_display(&entry.told_by)?;
                let marker = graph.marker_ref(entry.told_in.as_ref())?;
                if let Some(marker_text) =
                    visibility::entry_marker(&entry.visibility, &teller, Some(&marker))
                {
                    line.push(' ');
                    line.push_str(&marker_text);
                }
            }
            lines.push(format!("{line}\n"));
        }
        let header = "# Upcoming\n";
        if !lines.is_empty() && budget.take(char_len(header)) {
            let mut block = String::new();
            for line in lines {
                if !budget.take(char_len(&line)) {
                    break;
                }
                block.push_str(&line);
            }
            if block.is_empty() {
                budget.refund(char_len(header));
            } else {
                out.push_str(header);
                out.push_str(&block);
            }
        }
    }

    Ok(PackedBrief {
        text: out,
        plan: BriefPlan {
            full_participants,
            included_threads,
        },
    })
}

/// The number of Unicode scalar values in `text` — the unit `char_budget` counts, matching how the
/// entry text is measured elsewhere in the brief.
fn char_len(text: &str) -> usize {
    text.chars().count()
}

/// A running character budget for packing the brief. `take` admits an optional block only when it
/// fits; `charge` deducts mandatory content that renders regardless; `refund` returns a reserved
/// header when its section turned out empty.
struct Budget {
    remaining: usize,
}

impl Budget {
    fn new(chars: i64) -> Budget {
        Budget {
            remaining: chars.max(0) as usize,
        }
    }

    /// Admit `cost` if it fits the remaining budget, deducting it and returning `true`; otherwise leave
    /// the budget untouched and return `false`.
    fn take(&mut self, cost: usize) -> bool {
        if cost <= self.remaining {
            self.remaining -= cost;
            true
        } else {
            false
        }
    }

    /// Deduct mandatory content that renders regardless of the budget, saturating at zero.
    fn charge(&mut self, cost: usize) {
        self.remaining = self.remaining.saturating_sub(cost);
    }

    /// Return a previously `take`-n cost — used to un-reserve a section header whose body packed to
    /// nothing.
    fn refund(&mut self, cost: usize) {
        self.remaining += cost;
    }
}

mod helpers;
mod traced;

pub use traced::{BriefSectionTrace, BriefTrace, EntryTrace, SectionKind, compose_traced};

pub use helpers::{compose_participant, compose_participant_brief};

#[cfg(test)]
mod tests;
