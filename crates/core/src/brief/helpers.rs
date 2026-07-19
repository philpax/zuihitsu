use std::collections::HashSet;

use crate::{
    brief::{Brief, BriefError, BriefFact, BriefRelationship},
    decay,
    event::{Teller, Visibility},
    graph::{EntryView, Graph, MemoryView},
    ids::{MemoryId, MemoryName},
    settings::BriefSettings,
    time::Timestamp,
    visibility::{self, ClassOf},
    vocabulary::RelationName,
};

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
    match graph.memory_by_id(participant)? {
        Some(memory) => Ok(Some(memory_brief(
            graph,
            &memory,
            present_set,
            &class_of,
            settings,
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
    settings: &BriefSettings,
    now: Timestamp,
) -> Result<(), BriefError> {
    memory_brief(graph, memory, present_set, class_of, settings, now)?.render_body(out);
    Ok(())
}

/// Assemble a memory's [`Brief`] — subject, summary, visible recent facts, and relationships — the
/// structured form both [`compose`] (via [`render_memory_body`]) and the join path draw from. Reads
/// the depth bounds (`recent_facts`, `key_relationships`) from `settings` rather than taking them
/// pre-extracted, so a single argument carries the whole composition budget.
pub(super) fn memory_brief(
    graph: &Graph,
    memory: &MemoryView,
    present_set: &[MemoryId],
    class_of: &ClassOf,
    settings: &BriefSettings,
    now: Timestamp,
) -> Result<Brief, BriefError> {
    let recent = settings.recent_facts.max(0) as usize;
    let key_relationships = settings.key_relationships.max(0) as usize;
    Ok(Brief {
        subject: memory.name.clone(),
        summary: (!memory.description.is_empty()).then(|| memory.description.clone()),
        recent_facts: visible_recent_facts(graph, memory, present_set, class_of, recent, now)?,
        relationships: relationships(
            graph,
            memory.id,
            &memory.name,
            present_set,
            class_of,
            key_relationships,
        )?,
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
        facts.push(entry_fact(graph, memory, &entry, now)?);
    }
    let start = facts.len().saturating_sub(recent);
    Ok(facts.split_off(start))
}

/// Build a [`BriefFact`] for a single visible entry: its text with the provenance marker (resolving
/// its `told_in` room and `#confidential` flag) when non-public, followed by the staleness marker when
/// the entry has decayed on a `High`-volatility memory. Assumes the caller has already run the
/// visibility predicate — this only assembles the surviving entry's presentation.
fn entry_fact(
    graph: &Graph,
    memory: &MemoryView,
    entry: &EntryView,
    now: Timestamp,
) -> Result<BriefFact, BriefError> {
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
    Ok(BriefFact {
        text: entry.text.clone(),
        markers,
    })
}

/// The neighbour's most recent entry visible to `present_set` — the recency signal that ranks the edge.
/// It runs the *exact* same visibility predicate as a recent fact, so a confided aside on the neighbour
/// never sways the ranking for an audience that may not see it; `None` when the neighbour has no visible
/// entry at all.
fn latest_visible_entry(
    graph: &Graph,
    neighbour: &MemoryView,
    present_set: &[MemoryId],
    class_of: &ClassOf,
) -> Result<Option<EntryView>, BriefError> {
    let mut latest = None;
    for entry in graph.class_entries(neighbour.id)? {
        if visibility::visible(&entry, neighbour, present_set, class_of)? {
            latest = Some(entry);
        }
    }
    Ok(latest)
}

/// A memory's key relationships across its whole `same_as` class, as `source → relation → target`,
/// ranked by type-weight then recency and capped at `cap`. Reads [`Graph::class_neighbor_links`], so the
/// class is collapsed to one relationship set: the intra-class `same_as` plumbing (both endpoints in the
/// class) is dropped, and an external edge carried by more than one stub is shown once (deduplicated by
/// relation and neighbour). Both edges leaving the identity and edges running into it are surfaced, each
/// oriented by its stored direction — the neighbour is the source of an incoming edge and the target of
/// an outgoing one, with `subject` (this identity's handle) taking the other end — so the relationship
/// reads unambiguously rather than leaving this identity implicit. Soft-deleted neighbours are skipped
/// and each edge is filtered through `link_visible` when an audience is present. An `Attributed` link
/// carries a `[via teller]` provenance marker appended to the relationship line; a teller-private link
/// carries its marker the same way.
///
/// A hub memory can touch many live edges, so the list is ranked rather than dumped whole: the
/// structural, identity-bearing relations ([`relation_weight`]) float to the top and the high-volume
/// social edges (`knows`) fall away first when the list overflows `cap`. Ties within a weight break by
/// the neighbour's recency (its latest visible entry), then by relation label and endpoint handles, so
/// the order is fully deterministic.
fn relationships(
    graph: &Graph,
    id: MemoryId,
    subject: &MemoryName,
    present_set: &[MemoryId],
    class_of: &ClassOf,
    cap: usize,
) -> Result<Vec<BriefRelationship>, BriefError> {
    let mut ranked: Vec<RankedRelationship> = Vec::new();
    let mut seen: HashSet<(RelationName, MemoryId)> = HashSet::new();
    for link in graph.class_neighbor_links(id)? {
        // One relationship per (relation, neighbour): a class can carry the same external edge from
        // more than one of its stubs, and the collapsed block shows it once.
        if !seen.insert((link.relation.clone(), link.other)) {
            continue;
        }
        let symmetric = graph
            .relation(link.relation.as_str())?
            .map(|r| r.symmetric)
            .unwrap_or(false);
        if !visibility::link_visible(&link.link_vis(), symmetric, present_set, class_of)? {
            continue;
        }
        let Some(memory) = graph.memory_by_id(link.other)? else {
            continue;
        };
        let marker = if link.visibility != Visibility::Public {
            let teller = graph.teller_display(link.told_by.as_ref().unwrap_or(&Teller::Agent))?;
            let marker = graph.marker_ref(link.told_in.as_ref())?;
            visibility::link_marker(&link.visibility, &teller, Some(&marker))
        } else {
            None
        };
        // The neighbour's latest visible activity is the edge's recency signal; a neighbour with no
        // visible entry ranks last on recency (it still keeps its type-weight).
        let recency = latest_visible_entry(graph, &memory, present_set, class_of)?
            .map_or(i64::MIN, |entry| entry.asserted_at.as_millisecond());
        // Orient the edge against this identity: an incoming edge runs neighbour → subject, an outgoing
        // one runs subject → neighbour, so the rendered line names both ends in stored order.
        let (source, target) = if link.incoming {
            (memory.name.clone(), subject.clone())
        } else {
            (subject.clone(), memory.name.clone())
        };
        ranked.push(RankedRelationship {
            weight: relation_weight(&link.relation),
            recency,
            relationship: BriefRelationship {
                relation: link.relation.clone(),
                source,
                target,
                marker,
            },
        });
    }
    ranked.sort_by(|a, b| {
        b.weight
            .cmp(&a.weight)
            .then_with(|| b.recency.cmp(&a.recency))
            .then_with(|| {
                a.relationship
                    .relation
                    .as_str()
                    .cmp(b.relationship.relation.as_str())
            })
            .then_with(|| {
                a.relationship
                    .source
                    .as_str()
                    .cmp(b.relationship.source.as_str())
            })
            .then_with(|| {
                a.relationship
                    .target
                    .as_str()
                    .cmp(b.relationship.target.as_str())
            })
    });
    ranked.truncate(cap);
    Ok(ranked
        .into_iter()
        .map(|ranked| ranked.relationship)
        .collect())
}

/// A relationship paired with its ranking keys, so the edge list can be sorted before it is capped.
struct RankedRelationship {
    weight: u8,
    recency: i64,
    relationship: BriefRelationship,
}

/// The type-weight of a relation for brief ranking (higher sorts earlier). The ordering floats the
/// structural, identity-bearing relations the system seeds — origin, operatorship, composition,
/// participation, and placement (see `seed_relations`) — above the high-volume social edges, so a hub
/// memory's `created_by` survives a tight cap while its many `knows` edges are the first to fall away.
/// The weights are deliberately spaced constants gathered here so the ranking is legible and tunable
/// in one place; an agent-coined relation ([`RelationName::Other`]) sits mid-table, above bare
/// acquaintance but below the seeded structure.
fn relation_weight(relation: &RelationName) -> u8 {
    match relation {
        RelationName::CreatedBy | RelationName::Created => 100,
        RelationName::OperatorOf | RelationName::OperatedBy => 90,
        RelationName::PartOf | RelationName::Contains => 80,
        RelationName::ParticipatesIn | RelationName::HasParticipant => 70,
        RelationName::LocatedAt | RelationName::LocationOf => 60,
        RelationName::Other(_) => 50,
        RelationName::Knows | RelationName::KnownBy => 40,
        RelationName::SameAs => 10,
    }
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
            .map(|entry| entry.asserted_at.as_millisecond())
            .max()
            .unwrap_or(i64::MIN);
        keyed.push((latest, id));
    }
    keyed.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.0.cmp(&b.1.0)));
    Ok(keyed.into_iter().map(|(_, id)| id).collect())
}
