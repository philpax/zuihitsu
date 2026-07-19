use serde::{Deserialize, Serialize};

use crate::{
    event::Visibility,
    graph::{Graph, MemoryView},
    ids::{MemoryId, MemoryName},
    settings::BriefSettings,
    visibility::{self, ClassOf, VisibilityDecision},
    vocabulary::TagName,
};

use crate::brief::{BriefError, BriefRequest, compose_packed, helpers::ranked_present};

/// The composed brief plus its derivation: every memory the composer considered and, for each of
/// their entries, the visibility verdict and whether it reached the brief. Re-derived (not stored),
/// since composition is deterministic — this is the console's "how the brief was composed" surface.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct BriefTrace {
    /// The composed brief, identical to [`compose`]'s output.
    pub text: String,
    /// One per memory the composer drew from, in composition order.
    pub sections: Vec<BriefSectionTrace>,
}

/// One memory's contribution to the brief: which it is, the role it played, and the fate of each of
/// its entries.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct BriefSectionTrace {
    pub kind: SectionKind,
    pub memory: MemoryName,
    /// Only meaningful for a `CurrentRoom` section: whether the room carries `#confidential`.
    pub confidential: bool,
    pub entries: Vec<EntryTrace>,
}

/// The role a memory played in the brief (spec §Composition).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
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
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
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
    let packed = compose_packed(graph, settings, request)?;
    let plan = &packed.plan;
    let class_of = |id| graph.class_id(id).map(|class| class.unwrap_or(id));
    let recent = settings.recent_facts.max(0) as usize;
    let mut sections = Vec::new();

    // The self and current-room blocks are mandatory, so a surviving windowed entry always surfaces;
    // a participant or thread block surfaces its entries only when the budget admitted it as a full
    // block (an unadmitted one collapsed to name-only or dropped, so nothing on it reached the brief).
    if let Some(self_memory) = graph.self_memory()? {
        sections.push(section_trace(
            graph,
            &self_memory,
            request.present_set,
            &class_of,
            recent,
            SectionKind::SelfBrief,
            true,
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
            true,
        )?;
        section.confidential = context.tags.contains(&TagName::Confidential);
        sections.push(section);
    }

    // Speakers first, then the remaining present participants, matching the order [`compose_packed`]
    // renders them so the traced sections line up with the brief text.
    let ranked = ranked_present(graph, request.present_set)?;
    let is_speaker = |id: &MemoryId| request.speakers.contains(id);
    let ordered = ranked
        .iter()
        .filter(|id| is_speaker(id))
        .chain(ranked.iter().filter(|id| !is_speaker(id)));
    for &participant in ordered {
        if let Some(memory) = graph.memory_by_id(participant)? {
            sections.push(section_trace(
                graph,
                &memory,
                request.present_set,
                &class_of,
                recent,
                SectionKind::Participant,
                plan.full_participants.contains(&participant),
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
                plan.included_threads.contains(&id),
            )?);
        }
    }

    Ok(BriefTrace {
        text: packed.text,
        sections,
    })
}

/// Trace one memory's contribution. `surfaced` is whether the char budget admitted this block as a
/// full block: only then can a windowed entry be marked `in_brief`, since a block the budget did not
/// admit contributed nothing to the brief text.
fn section_trace(
    graph: &Graph,
    memory: &MemoryView,
    present_set: &[MemoryId],
    class_of: &ClassOf,
    recent: usize,
    kind: SectionKind,
    surfaced: bool,
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
    if surfaced {
        let keep_from = visible_positions.len().saturating_sub(recent);
        for &position in &visible_positions[keep_from..] {
            entries[position].in_brief = true;
        }
    }
    Ok(BriefSectionTrace {
        kind,
        memory: memory.name.clone(),
        confidential: false,
        entries,
    })
}
