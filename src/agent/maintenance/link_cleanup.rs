//! The link-redundant entry cleanup pass: retracts entries whose content is purely a
//! description of a link that exists.
//!
//! Per identity class with content changes since the cursor:
//! 1. Gather live entries and existing links, dropping connector-maintained entries — the connector
//!    owns those and this pass must never mutate them.
//! 2. Call the model to identify entries whose content is purely a description of a link (no
//!    textured detail beyond the link's structural assertion).
//! 3. Retract each identified entry through the [`MemoryBlock`] write path under [`Authority::Agent`],
//!    stamping the pass's own template as provenance — so the retraction clears the same guards a
//!    turn's retraction does rather than appending a raw event.

use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    InstanceError,
    agent::{TurnError, templates},
    engine::Engine,
    event::{EventSource, ModelPhase, ProducedBy, PromptTemplateName, Teller},
    graph::EntryView,
    ids::{MemoryId, Seq, TurnId},
    memory::memory_block::{Authority, MemoryBlock},
    model::{GenerateRequest, ModelClient},
    settings::{CaptureLevel, Settings},
};

use crate::agent::turn::{Recording, collect_written_memories};

/// Run one link-cleanup sweep. Returns `(new_cursor, memories_considered)`.
pub async fn catch_up(
    engine: &Arc<Engine>,
    model: &dyn ModelClient,
    cursor: Seq,
) -> Result<(Seq, usize), InstanceError> {
    let head = engine.store.lock().head()?;
    if head <= cursor {
        return Ok((cursor, 0));
    }

    let Some(template) = templates::latest_template(
        engine.store.lock().as_ref(),
        PromptTemplateName::LinkCleanup,
    )?
    else {
        return Ok((head, 0));
    };

    let written = collect_written_memories(engine.store.lock().as_ref(), cursor)?;
    if written.is_empty() {
        return Ok((head, 0));
    }

    let recording = Recording::new(None, TurnId::generate(), CaptureLevel::Off);
    let produced_by = ProducedBy {
        model_id: model.model_id().into(),
        template_name: PromptTemplateName::LinkCleanup,
        template_version: template.version,
    };
    let max_entry_chars = Settings::from_store(engine.store.lock().as_ref())
        .map(|s| s.memory.max_entry_chars.max(1) as usize)
        .unwrap_or(1);
    let mut block = MemoryBlock::new(
        engine.clone(),
        Teller::Agent,
        Authority::Agent,
        None,
        None,
        Vec::new(),
        max_entry_chars,
    )?;

    for &id in &written {
        let (entries, links) = {
            let graph = engine.graph.lock();
            let entries = graph.class_entries(id)?;
            let links = graph.class_links(id)?;
            (entries, links)
        };
        // A connector-maintained entry is never a cleanup candidate: the connector owns its id and
        // supersedes or retracts it as the platform-side account changes, so this pass leaves it out
        // of both the prompt and any retraction.
        let candidates: Vec<EntryView> = entries
            .into_iter()
            .filter(|entry| !entry.origin.is_connector())
            .collect();
        if candidates.is_empty() {
            continue;
        }

        match identify_redundant(
            engine,
            model,
            &recording,
            &template.body,
            id,
            &candidates,
            &links,
        )
        .await
        {
            Ok(redundant_ids) => {
                for entry_id in redundant_ids {
                    if let Err(error) =
                        block.retract(id, entry_id, RETRACTION_REASON, Some(produced_by.clone()))
                    {
                        tracing::warn!(
                            memory = ?id,
                            %error,
                            "link cleanup: retraction rejected; skipping entry"
                        );
                    }
                }
            }
            Err(error) => {
                tracing::warn!(
                    memory = ?id,
                    %error,
                    "link cleanup: identification failed; skipping"
                );
            }
        }
    }

    let events = block.into_effects().events;
    if !events.is_empty() {
        let now = engine.clock.now();
        engine
            .store
            .lock()
            .append(now, EventSource::Orchestration, events)?;
        engine
            .graph
            .lock()
            .materialize_from(engine.store.lock().as_ref())?;
    }

    Ok((head, written.len()))
}

/// The reason recorded on every link-redundant retraction.
const RETRACTION_REASON: &str = "content fully captured by existing link";

/// The model's response: the set of entry ids to retract.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct RedundantEntries {
    #[serde(default)]
    retract_entry_ids: Vec<String>,
}

/// Call the model to identify entries whose content is purely a description of a link, returning the
/// resolved entry ids (only those present among `entries`).
async fn identify_redundant(
    engine: &Engine,
    model: &dyn ModelClient,
    recording: &Recording,
    template_body: &str,
    memory_id: MemoryId,
    entries: &[EntryView],
    links: &[crate::graph::ClassLinkView],
) -> Result<Vec<crate::ids::EntryId>, InstanceError> {
    let entry_lines: Vec<String> = entries
        .iter()
        .map(|e| format!("- id: {}, text: {:?}", e.entry_id.0, e.text))
        .collect();
    let link_lines: Vec<String> = {
        let graph = engine.graph.lock();
        links
            .iter()
            .map(|l| {
                let to_name = graph
                    .memory_by_id(l.to)
                    .ok()
                    .flatten()
                    .map(|m| m.name.as_str().to_owned())
                    .unwrap_or_else(|| l.to.0.to_string());
                format!("- {} → {}", l.relation.as_str(), to_name)
            })
            .collect()
    };
    let user_prompt = format!(
        "Memory: {}\n\nEntries:\n{}\n\nExisting links:\n{}",
        memory_id.0,
        entry_lines.join("\n"),
        if link_lines.is_empty() {
            "(none)".to_owned()
        } else {
            link_lines.join("\n")
        }
    );

    let request = GenerateRequest::structured::<RedundantEntries>(
        template_body,
        user_prompt,
        "redundant_entries",
    );

    let record = recording.request_record(&request, None, &[]);
    let response = recording
        .generate(engine, model, &request, ModelPhase::Synthesis, record, None)
        .await
        .map_err(|e| InstanceError::from(TurnError::Model(e)))?
        .expect_completed();

    let Some(parsed) = crate::model::parse_structured::<RedundantEntries>(&response.completion)
    else {
        return Ok(Vec::new());
    };

    // Parse the entry ids from strings back to EntryId, keeping only those that name a candidate.
    let entry_ids: Vec<crate::ids::EntryId> = parsed
        .retract_entry_ids
        .iter()
        .filter_map(|id_str| {
            ulid::Ulid::from_string(id_str)
                .ok()
                .map(crate::ids::EntryId)
                .filter(|id| entries.iter().any(|e| e.entry_id == *id))
        })
        .collect();

    Ok(entry_ids)
}
