use crate::{
    decay,
    event::Visibility,
    graph::{Graph, MemoryView},
    ids::MemoryId,
    settings::BriefSettings,
    time::Timestamp,
    visibility::{self, ClassOf},
};

use super::{Brief, BriefError, BriefFact, BriefRelationship};

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
pub(super) fn render_memory_body(
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
        relationships: relationships(graph, memory.id, present_set, class_of)?,
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
            let marker = graph.marker_ref(entry.told_in.as_ref())?;
            if let Some(marker_text) =
                visibility::entry_marker(&entry.visibility, &teller, Some(&marker))
            {
                markers.push(marker_text);
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

/// A memory's key relationships, as `relation → other-handle`, skipping soft-deleted neighbours and
/// filtering through `link_visible` when an audience is present. An `Attributed` link carries a
/// `[via teller]` provenance marker appended to the relationship line; a teller-private link carries
/// its marker the same way. The full ranking by recency × type-weight (spec §Per-participant brief)
/// is a later refinement; this lists the live edges touching the memory.
fn relationships(
    graph: &Graph,
    id: MemoryId,
    present_set: &[MemoryId],
    class_of: &ClassOf,
) -> Result<Vec<BriefRelationship>, BriefError> {
    let mut relationships = Vec::new();
    for link in graph.links(id)? {
        let symmetric = graph
            .relation(link.relation.as_str())?
            .map(|r| r.symmetric)
            .unwrap_or(false);
        if !visibility::link_visible(&link.link_vis(), symmetric, present_set, class_of)? {
            continue;
        }
        let other = if link.from == id { link.to } else { link.from };
        if let Some(memory) = graph.memory_by_id(other)? {
            let marker = if link.visibility != Visibility::Public {
                let teller = graph.teller_display(
                    link.told_by
                        .as_ref()
                        .unwrap_or(&crate::event::Teller::Agent),
                )?;
                let marker = graph.marker_ref(link.told_in.as_ref())?;
                visibility::link_marker(&link.visibility, &teller, Some(&marker))
            } else {
                None
            };
            relationships.push(BriefRelationship {
                relation: link.relation.clone(),
                subject: memory.name.clone(),
                marker,
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
pub(super) fn ranked_present(
    graph: &Graph,
    present_set: &[MemoryId],
) -> Result<Vec<MemoryId>, BriefError> {
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
