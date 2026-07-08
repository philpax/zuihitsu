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

use std::fmt::Write as _;

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
pub struct Brief {
    /// The memory this block is about — the `## <subject>` header of the rendered form.
    pub subject: MemoryName,
    /// The memory's prose description, absent when it has none.
    pub summary: Option<String>,
    /// The visible recent facts, oldest first, in the same recency window the composer applies.
    pub recent_facts: Vec<BriefFact>,
    /// The memory's key relationships, each `relation → subject`.
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

/// One relationship in a [`Brief`]: the relation label and the neighbour it points to, rendered as
/// `relation: subject [marker]`. `relation` serializes as its bare label (the wire form
/// [`RelationName`] keeps), so it is typed as a `string` on the console side. `marker` carries the
/// provenance marker for a non-public link (`[via Erin]` or `[teller-private, …]`), appended after
/// the relationship line since a link has no text body.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct BriefRelationship {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub relation: RelationName,
    pub subject: MemoryName,
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
                    "- {}: {}",
                    relationship.relation.as_str(),
                    relationship.subject.as_str()
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
pub fn compose(
    graph: &Graph,
    settings: &BriefSettings,
    request: &BriefRequest,
) -> Result<String, BriefError> {
    let &BriefRequest {
        present_set,
        current_context,
        working_set,
        now,
    } = request;
    // The visibility predicate resolves identity over the `same_as` class.
    let class_of = |id| graph.class_id(id).map(|class| class.unwrap_or(id));
    let recent = settings.recent_facts.max(0) as usize;
    let mut out = String::new();

    // 1. Self brief — the agent's own memory in the per-participant shape.
    if let Some(self_memory) = graph.self_memory()? {
        out.push_str("# You\n");
        render_memory_body(
            &mut out,
            graph,
            &self_memory,
            present_set,
            &class_of,
            recent,
            now,
        )?;
        out.push('\n');
    }

    // 2. Current context — the room — with its #confidential tag, visible regardless of who is
    //    present (it is a memory-level tag, not a teller-gated entry).
    if let Some(context_id) = current_context
        && let Some(context) = graph.memory_by_id(context_id)?
    {
        let room = visibility::room_display(context.name.as_str());
        if context.tags.contains(&TagName::Confidential) {
            let _ = writeln!(out, "# Current room: {room} (confidential)");
        } else {
            let _ = writeln!(out, "# Current room: {room}");
        }
        if !context.description.is_empty() {
            let _ = writeln!(out, "{}", context.description);
        }
        out.push('\n');
    }

    // 3. Per-participant briefs, ranked by recency and capped. The cap bounds full blocks only; the
    //    predicate above and below always sees the full present set.
    if !present_set.is_empty() {
        out.push_str("# Present\n");
        let cap = settings.present_set_cap.max(0) as usize;
        for (index, participant) in ranked_present(graph, present_set)?.into_iter().enumerate() {
            let Some(memory) = graph.memory_by_id(participant)? else {
                continue;
            };
            if index < cap {
                let _ = writeln!(out, "## {}", memory.name.as_str());
                render_memory_body(
                    &mut out,
                    graph,
                    &memory,
                    present_set,
                    &class_of,
                    recent,
                    now,
                )?;
            } else {
                // The tail collapses to name-only — still present for the predicate, just not
                // given a full block (spec §Present-set cap).
                let _ = writeln!(out, "- {} (present)", memory.name.as_str());
            }
        }
    }

    // 6. Active threads — the working set carried across a compaction seam: the memories the ending
    //    session touched, re-surfaced so the new session does not lose the thread. Each is rendered
    //    in the per-participant shape, so its facts are re-filtered through `visible` against the new
    //    present set (an aside about a now-present subject is suppressed). Self, the current room, and
    //    present participants are already shown above, so they are skipped to avoid duplication.
    if !working_set.is_empty() {
        let self_id = graph.self_memory()?.map(|memory| memory.id);
        let mut threads = String::new();
        for &id in working_set {
            if Some(id) == self_id || Some(id) == current_context || present_set.contains(&id) {
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
                recent,
                now,
            )?;
            if body.trim().is_empty() {
                continue;
            }
            let _ = writeln!(threads, "## {}", memory.name.as_str());
            threads.push_str(&body);
        }
        if !threads.is_empty() {
            out.push_str("# Active threads\n");
            out.push_str(&threads);
        }
    }

    // 7. Upcoming — near-future calendared items, soonest first, so the agent organically raises
    //    them (spec §Calendar → <upcoming/>). Each occurrence is filtered through `visible` like any
    //    entry — a private aside about an absent person carries its marker, one about a now-present
    //    subject is suppressed — and the list is capped.
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
                let room = graph.marker_room(entry.told_in)?;
                if let Some(marker) =
                    visibility::entry_marker(&entry.visibility, &teller, room.as_ref())
                {
                    line.push(' ');
                    line.push_str(&marker);
                }
            }
            lines.push(line);
        }
        if !lines.is_empty() {
            out.push_str("# Upcoming\n");
            for line in lines {
                let _ = writeln!(out, "{line}");
            }
        }
    }

    Ok(out)
}

mod helpers;
mod traced;

pub use traced::{BriefSectionTrace, BriefTrace, EntryTrace, SectionKind, compose_traced};

pub use helpers::{compose_participant, compose_participant_brief};

#[cfg(test)]
mod tests;
