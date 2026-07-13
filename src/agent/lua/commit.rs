//! The commit summary — folding what a block actually changed into the result the agent reads.

use std::collections::BTreeMap;

use crate::{engine::Engine, event::EventPayload, ids::MemoryId};

/// Fold a block's committed-effects summary into the result the agent reads. A write-only block returns
/// `nil`, which on its own tells the agent nothing about whether its create or append landed — so the
/// summary stands in for a bare `nil`/empty result, and trails a genuine returned value otherwise.
pub(super) fn with_commit_summary(rendered: String, summary: Option<String>) -> String {
    let Some(summary) = summary else {
        return rendered;
    };
    let trimmed = rendered.trim();
    if trimmed.is_empty() || trimmed == "nil" {
        summary
    } else {
        format!("{rendered}\n\n{summary}")
    }
}

/// A concise, agent-facing summary of what a block committed — "Committed: created topic/q3_plan;
/// appended 2 entries to topic/q3_plan." — built from the block's effect events. `None` when the block
/// changed nothing the agent should be told about (a read-only query), so a pure read keeps its own
/// rendered result unchanged. Names of memories created in this very block are read from the events
/// (they are not in the graph yet); existing targets of an append or link resolve through the graph.
pub(super) fn summarize_committed(engine: &Engine, events: &[EventPayload]) -> Option<String> {
    let fresh: BTreeMap<MemoryId, String> = events
        .iter()
        .filter_map(|event| match event {
            EventPayload::MemoryCreated { id, name } => Some((*id, name.as_str().to_owned())),
            _ => None,
        })
        .collect();
    let name_of = |id: MemoryId| -> String {
        fresh
            .get(&id)
            .cloned()
            .or_else(|| {
                engine
                    .graph
                    .lock()
                    .memory_by_id(id)
                    .ok()
                    .flatten()
                    .map(|memory| memory.name.as_str().to_owned())
            })
            .unwrap_or_else(|| "a memory".to_owned())
    };

    let mut created: Vec<String> = Vec::new();
    let mut appended: BTreeMap<MemoryId, usize> = BTreeMap::new();
    let mut superseded: Vec<MemoryId> = Vec::new();
    let mut retracted: Vec<MemoryId> = Vec::new();
    let mut other: Vec<String> = Vec::new();
    for event in events {
        match event {
            EventPayload::MemoryCreated { name, .. } => created.push(name.as_str().to_owned()),
            EventPayload::MemoryContentAppended { id, .. } => {
                *appended.entry(*id).or_default() += 1
            }
            EventPayload::MemorySuperseded { id, .. } => superseded.push(*id),
            EventPayload::EntryRetracted { memory, .. } => retracted.push(*memory),
            EventPayload::LinkCreated {
                from, to, relation, ..
            } => other.push(format!(
                "linked {} {} {}",
                name_of(*from),
                relation.as_str(),
                name_of(*to)
            )),
            EventPayload::LinkRemoved {
                from, to, relation, ..
            } => other.push(format!(
                "removed the {} link between {} and {}",
                relation.as_str(),
                name_of(*from),
                name_of(*to)
            )),
            // A proposal is inert until the adjudication pass weighs it, so the summary says what
            // actually happened — a proposal, not a merge — for the agent's reply to stay honest.
            EventPayload::MergeProposed { from, to, .. } => other.push(format!(
                "proposed merging {} into {} — a merge lands only when adjudicated",
                name_of(*from),
                name_of(*to)
            )),
            EventPayload::TagAppliedToMemory { memory, tag } => {
                other.push(format!("tagged {} #{}", name_of(*memory), tag.as_str()))
            }
            EventPayload::TagRemovedFromMemory { memory, tag } => other.push(format!(
                "removed #{} from {}",
                tag.as_str(),
                name_of(*memory)
            )),
            EventPayload::TagCreated { name, .. } => {
                other.push(format!("created the tag #{}", name.as_str()))
            }
            EventPayload::MemoryDeleted { id } => other.push(format!("deleted {}", name_of(*id))),
            EventPayload::MemoryRenamed {
                old_name, new_name, ..
            } => other.push(format!(
                "renamed {} to {}",
                old_name.as_str(),
                new_name.as_str()
            )),
            EventPayload::LinkTypeRegistered { name, .. } => {
                other.push(format!("registered the {} relation", name.as_str()))
            }
            _ => {}
        }
    }

    let mut summary: Vec<String> = Vec::new();
    if !created.is_empty() {
        summary.push(format!("created {}", created.join(", ")));
    }
    for (id, count) in &appended {
        let entries = if *count == 1 { "entry" } else { "entries" };
        summary.push(format!("appended {count} {entries} to {}", name_of(*id)));
    }
    for id in &superseded {
        summary.push(format!("superseded an entry on {}", name_of(*id)));
    }
    for id in &retracted {
        summary.push(format!("retracted an entry on {}", name_of(*id)));
    }
    summary.extend(other);

    (!summary.is_empty()).then(|| format!("Committed: {}.", summary.join("; ")))
}
