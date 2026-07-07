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
    graph::{Graph, GraphError, MemoryView},
    ids::{MemoryId, MemoryName},
    settings::BriefSettings,
    time::{self, Timestamp},
    vocabulary::{RelationName, TagName},
};

use crate::{
    decay,
    visibility::{self, ClassOf, VisibilityDecision},
};

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
/// `relation: subject`. `relation` serializes as its bare label (the wire form [`RelationName`] keeps),
/// so it is typed as a `string` on the console side.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct BriefRelationship {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub relation: RelationName,
    pub subject: MemoryName,
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
                let _ = writeln!(
                    out,
                    "- {}: {}",
                    relationship.relation.as_str(),
                    relationship.subject.as_str()
                );
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

/// The composed brief plus its derivation: every memory the composer considered and, for each of
/// their entries, the visibility verdict and whether it reached the brief. Re-derived (not stored),
/// since composition is deterministic — this is the console's "how the brief was composed" surface.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BriefTrace {
    /// The composed brief, identical to [`compose`]'s output.
    pub text: String,
    /// One per memory the composer drew from, in composition order.
    pub sections: Vec<BriefSectionTrace>,
}

/// One memory's contribution to the brief: which it is, the role it played, and the fate of each of
/// its entries.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BriefSectionTrace {
    pub kind: SectionKind,
    pub memory: MemoryName,
    /// Only meaningful for a `CurrentRoom` section: whether the room carries `#confidential`.
    pub confidential: bool,
    pub entries: Vec<EntryTrace>,
}

/// The role a memory played in the brief (spec §Composition).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SectionKind {
    /// The agent's own `self` memory.
    SelfBrief,
    /// The current room's [`Namespace::Context`] memory.
    CurrentRoom,
    /// A present participant.
    Participant,
    /// A working-set memory carried across a compaction seam.
    ActiveThread,
}

/// One entry's fate during composition: its text, its declared visibility, the predicate's verdict,
/// and whether — given the verdict and the recency window — it actually reached the brief.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EntryTrace {
    pub text: String,
    pub visibility: Visibility,
    pub decision: VisibilityDecision,
    /// True when the entry both passed the predicate and fell within the recent-facts window.
    pub in_brief: bool,
}

/// Compose the brief and, alongside it, the trace of how it was built: every considered memory and
/// the per-entry verdicts. The text matches [`compose`] exactly; the sections walk the same memories
/// in the same order, recording for each entry why it did or did not surface.
pub fn compose_traced(
    graph: &Graph,
    settings: &BriefSettings,
    request: &BriefRequest,
) -> Result<BriefTrace, BriefError> {
    let text = compose(graph, settings, request)?;
    let class_of = |id| graph.class_id(id).map(|class| class.unwrap_or(id));
    let recent = settings.recent_facts.max(0) as usize;
    let mut sections = Vec::new();

    if let Some(self_memory) = graph.self_memory()? {
        sections.push(section_trace(
            graph,
            &self_memory,
            request.present_set,
            &class_of,
            recent,
            SectionKind::SelfBrief,
        )?);
    }

    if let Some(context_id) = request.current_context
        && let Some(context) = graph.memory_by_id(context_id)?
    {
        let mut section = section_trace(
            graph,
            &context,
            request.present_set,
            &class_of,
            recent,
            SectionKind::CurrentRoom,
        )?;
        section.confidential = context.tags.contains(&TagName::Confidential);
        sections.push(section);
    }

    for participant in ranked_present(graph, request.present_set)? {
        if let Some(memory) = graph.memory_by_id(participant)? {
            sections.push(section_trace(
                graph,
                &memory,
                request.present_set,
                &class_of,
                recent,
                SectionKind::Participant,
            )?);
        }
    }

    let self_id = graph.self_memory()?.map(|memory| memory.id);
    for &id in request.working_set {
        if Some(id) == self_id
            || Some(id) == request.current_context
            || request.present_set.contains(&id)
        {
            continue;
        }
        if let Some(memory) = graph.memory_by_id(id)? {
            sections.push(section_trace(
                graph,
                &memory,
                request.present_set,
                &class_of,
                recent,
                SectionKind::ActiveThread,
            )?);
        }
    }

    Ok(BriefTrace { text, sections })
}

/// Compose a single participant's brief block, against `present_set`. Used for a mid-session join:
/// the joiner's brief is built against the now-present set and injected as a system message, rather
/// than rebuilding the whole frozen prompt (spec §Mid-conversation joins). Because `present_set`
/// includes the joiner, the subject-guard suppresses asides about them — a subject joining closes
/// the dangerous direction. Empty if the participant is unknown.
pub fn compose_participant(
    graph: &Graph,
    participant: MemoryId,
    present_set: &[MemoryId],
    settings: &BriefSettings,
    now: Timestamp,
) -> Result<String, BriefError> {
    Ok(
        match compose_participant_brief(graph, participant, present_set, settings, now)? {
            Some(brief) => brief.render(),
            None => String::new(),
        },
    )
}

/// Compose a single participant's [`Brief`] as structured data, against `present_set`. The structured
/// form a mid-session join carries on its `system` turn: the caller renders it to markup for the
/// agent's prompt (via [`Brief::render`], the projection [`compose_participant`] returns) and carries
/// the struct itself for structured consumers. `None` when the participant is unknown — the same empty
/// result the string form yields.
pub fn compose_participant_brief(
    graph: &Graph,
    participant: MemoryId,
    present_set: &[MemoryId],
    settings: &BriefSettings,
    now: Timestamp,
) -> Result<Option<Brief>, BriefError> {
    let class_of = |id| graph.class_id(id).map(|class| class.unwrap_or(id));
    let recent = settings.recent_facts.max(0) as usize;
    match graph.memory_by_id(participant)? {
        Some(memory) => Ok(Some(memory_brief(
            graph,
            &memory,
            present_set,
            &class_of,
            recent,
            now,
        )?)),
        None => Ok(None),
    }
}

/// Render a memory's body in the per-participant shape: summary, visible recent facts (with the
/// teller-private marker beside the text), and key relationships. Delegates to [`Brief::render_body`]
/// so the composer and the join path project identical markup from the same structured source.
fn render_memory_body(
    out: &mut String,
    graph: &Graph,
    memory: &MemoryView,
    present_set: &[MemoryId],
    class_of: &ClassOf,
    recent: usize,
    now: Timestamp,
) -> Result<(), BriefError> {
    memory_brief(graph, memory, present_set, class_of, recent, now)?.render_body(out);
    Ok(())
}

/// Assemble a memory's [`Brief`] — subject, summary, visible recent facts, and relationships — the
/// structured form both [`compose`] (via [`render_memory_body`]) and the join path draw from.
fn memory_brief(
    graph: &Graph,
    memory: &MemoryView,
    present_set: &[MemoryId],
    class_of: &ClassOf,
    recent: usize,
    now: Timestamp,
) -> Result<Brief, BriefError> {
    Ok(Brief {
        subject: memory.name.clone(),
        summary: (!memory.description.is_empty()).then(|| memory.description.clone()),
        recent_facts: visible_recent_facts(graph, memory, present_set, class_of, recent, now)?,
        relationships: relationships(graph, memory.id)?,
    })
}

/// A memory's last `recent` content entries that are visible to `present_set`, in commit order, each
/// carrying the inline teller-private marker when it is a surviving private entry (resolving its
/// `told_in` room and `#confidential` flag at build time) and the staleness marker when decayed.
fn visible_recent_facts(
    graph: &Graph,
    memory: &MemoryView,
    present_set: &[MemoryId],
    class_of: &ClassOf,
    recent: usize,
    now: Timestamp,
) -> Result<Vec<BriefFact>, BriefError> {
    let mut facts = Vec::new();
    for entry in graph.class_entries(memory.id)? {
        if !visibility::visible(&entry, memory, present_set, class_of)? {
            continue;
        }
        let mut markers = Vec::new();
        if entry.visibility != Visibility::Public {
            let teller = graph.teller_display(&entry.told_by)?;
            let room = graph.marker_room(entry.told_in)?;
            if let Some(marker) =
                visibility::entry_marker(&entry.visibility, &teller, room.as_ref())
            {
                markers.push(marker);
            }
        }
        let effective = entry.occurred_sort.unwrap_or(entry.asserted_at);
        if decay::is_stale(memory.volatility, effective, now) {
            markers.push(decay::STALE_MARKER.to_owned());
        }
        facts.push(BriefFact {
            text: entry.text.clone(),
            markers,
        });
    }
    let start = facts.len().saturating_sub(recent);
    Ok(facts.split_off(start))
}

/// A memory's key relationships, as `relation → other-handle`, skipping soft-deleted neighbours. The
/// full ranking by recency × type-weight (spec §Per-participant brief) is a later refinement; this
/// lists the live edges touching the memory.
fn relationships(graph: &Graph, id: MemoryId) -> Result<Vec<BriefRelationship>, BriefError> {
    let mut relationships = Vec::new();
    for link in graph.links(id)? {
        let other = if link.from == id { link.to } else { link.from };
        if let Some(memory) = graph.memory_by_id(other)? {
            relationships.push(BriefRelationship {
                relation: link.relation.clone(),
                subject: memory.name.clone(),
            });
        }
    }
    Ok(relationships)
}

/// The present participants ordered for the cap: most-recently-active first (by their latest
/// asserted entry across the merged identity), with the memory id as a deterministic tie-break.
/// Trace one memory's entries: walk its full history (superseded entries included, so a fact
/// filtered for being superseded still appears with its reason), record each entry's visibility
/// verdict, and mark the last `recent` surviving entries as the ones that reached the brief — the
/// same recency window the composer applies.
fn section_trace(
    graph: &Graph,
    memory: &MemoryView,
    present_set: &[MemoryId],
    class_of: &ClassOf,
    recent: usize,
    kind: SectionKind,
) -> Result<BriefSectionTrace, BriefError> {
    let mut entries = Vec::new();
    let mut visible_positions = Vec::new();
    for entry in graph.class_history(memory.id)? {
        let decision = visibility::explain(&entry, memory, present_set, class_of)?;
        if decision.is_visible() {
            visible_positions.push(entries.len());
        }
        entries.push(EntryTrace {
            text: entry.text,
            visibility: entry.visibility,
            decision,
            in_brief: false,
        });
    }
    let keep_from = visible_positions.len().saturating_sub(recent);
    for &position in &visible_positions[keep_from..] {
        entries[position].in_brief = true;
    }
    Ok(BriefSectionTrace {
        kind,
        memory: memory.name.clone(),
        confidential: false,
        entries,
    })
}

fn ranked_present(graph: &Graph, present_set: &[MemoryId]) -> Result<Vec<MemoryId>, BriefError> {
    let mut keyed: Vec<(i64, MemoryId)> = Vec::new();
    for &id in present_set {
        let latest = graph
            .class_entries(id)?
            .iter()
            .map(|entry| entry.asserted_at.as_millis())
            .max()
            .unwrap_or(i64::MIN);
        keyed.push((latest, id));
    }
    keyed.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.0.cmp(&b.1.0)));
    Ok(keyed.into_iter().map(|(_, id)| id).collect())
}

#[cfg(test)]
mod tests;
