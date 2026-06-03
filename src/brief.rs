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

use crate::{
    event::Visibility,
    graph::{Graph, GraphError, MemoryView},
    ids::{MemoryId, MemoryName, TagName},
    settings::BriefSettings,
    visibility::{self, ClassOf},
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

/// Compose the contextual brief for `present_set` in the room `current_context`. The predicate
/// always resolves against the full `present_set`; `settings.present_set_cap` bounds only how many
/// participants get a full block, with the remainder collapsed to name-only. `working_set` is the
/// memories carried across a compaction seam (empty otherwise), rendered as an active-threads
/// section so continuity holds — re-filtered through `visible` against the present set like any other
/// block (spec §Compaction → working-set carryover).
pub fn compose(
    graph: &Graph,
    present_set: &[MemoryId],
    current_context: Option<MemoryId>,
    settings: &BriefSettings,
    working_set: &[MemoryId],
) -> Result<String, BriefError> {
    // The visibility predicate resolves identity over the `same_as` class.
    let class_of = |id| graph.class_id(id).map(|class| class.unwrap_or(id));
    let recent = settings.recent_facts.max(0) as usize;
    let mut out = String::new();

    // 1. Self brief — the agent's own memory in the per-participant shape.
    if let Some(self_memory) = graph.memory_by_name(MemoryName::SELF)? {
        out.push_str("# You\n");
        render_memory_body(
            &mut out,
            graph,
            &self_memory,
            present_set,
            &class_of,
            recent,
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
                render_memory_body(&mut out, graph, &memory, present_set, &class_of, recent)?;
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
        let self_id = graph
            .memory_by_name(MemoryName::SELF)?
            .map(|memory| memory.id);
        let mut threads = String::new();
        for &id in working_set {
            if Some(id) == self_id || Some(id) == current_context || present_set.contains(&id) {
                continue;
            }
            let Some(memory) = graph.memory_by_id(id)? else {
                continue;
            };
            let mut body = String::new();
            render_memory_body(&mut body, graph, &memory, present_set, &class_of, recent)?;
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

    Ok(out)
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
) -> Result<String, BriefError> {
    let class_of = |id| graph.class_id(id).map(|class| class.unwrap_or(id));
    let recent = settings.recent_facts.max(0) as usize;
    let mut out = String::new();
    if let Some(memory) = graph.memory_by_id(participant)? {
        let _ = writeln!(out, "## {}", memory.name.as_str());
        render_memory_body(&mut out, graph, &memory, present_set, &class_of, recent)?;
    }
    Ok(out)
}

/// Render a memory's body in the per-participant shape: summary, visible recent facts (with the
/// teller-private marker baked in), and key relationships.
fn render_memory_body(
    out: &mut String,
    graph: &Graph,
    memory: &MemoryView,
    present_set: &[MemoryId],
    class_of: &ClassOf,
    recent: usize,
) -> Result<(), BriefError> {
    if !memory.description.is_empty() {
        let _ = writeln!(out, "<summary>{}</summary>", memory.description);
    }

    let facts = visible_recent_facts(graph, memory, present_set, class_of, recent)?;
    if !facts.is_empty() {
        out.push_str("<recent_facts>\n");
        for fact in &facts {
            let _ = writeln!(out, "- {fact}");
        }
        out.push_str("</recent_facts>\n");
    }

    let relationships = relationships(graph, memory.id)?;
    if !relationships.is_empty() {
        out.push_str("<relationships>\n");
        for relationship in &relationships {
            let _ = writeln!(out, "- {relationship}");
        }
        out.push_str("</relationships>\n");
    }

    Ok(())
}

/// A memory's last `recent` content entries that are visible to `present_set`, in commit order, each
/// rendered as text with the inline teller-private marker appended when it is a surviving private
/// entry (resolving its `told_in` room and `#confidential` flag at build time).
fn visible_recent_facts(
    graph: &Graph,
    memory: &MemoryView,
    present_set: &[MemoryId],
    class_of: &ClassOf,
    recent: usize,
) -> Result<Vec<String>, BriefError> {
    let mut facts = Vec::new();
    for entry in graph.class_entries(memory.id)? {
        if !visibility::visible(&entry, memory, present_set, class_of)? {
            continue;
        }
        let mut line = entry.text.clone();
        if entry.visibility != Visibility::Public {
            let teller = graph.teller_display(&entry.told_by)?;
            let room = graph.marker_room(entry.told_in)?;
            line.push(' ');
            line.push_str(&visibility::teller_private_marker(&teller, room.as_ref()));
        }
        facts.push(line);
    }
    let start = facts.len().saturating_sub(recent);
    Ok(facts.split_off(start))
}

/// A memory's key relationships, rendered as `relation: other-handle`, skipping soft-deleted
/// neighbours. The full ranking by recency × type-weight (spec §Per-participant brief) is a later
/// refinement; this lists the live edges touching the memory.
fn relationships(graph: &Graph, id: MemoryId) -> Result<Vec<String>, BriefError> {
    let mut relationships = Vec::new();
    for link in graph.links(id)? {
        let other = if link.from == id { link.to } else { link.from };
        if let Some(memory) = graph.memory_by_id(other)? {
            relationships.push(format!(
                "{}: {}",
                link.relation.as_str(),
                memory.name.as_str()
            ));
        }
    }
    Ok(relationships)
}

/// The present participants ordered for the cap: most-recently-active first (by their latest
/// asserted entry across the merged identity), with the memory id as a deterministic tie-break.
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
