//! The link-redundant entry cleanup pass: retracts entries whose content is purely a
//! description of a link that exists.
//!
//! Per identity class with content changes since the cursor:
//! 1. Gather live entries and existing links.
//! 2. Call the model to identify entries whose content is purely a description of a link (no
//!    textured detail beyond the link's structural assertion).
//! 3. Retract each identified entry with `EntryRetracted`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    InstanceError,
    agent::{TurnError, templates},
    engine::Engine,
    event::{EventPayload, EventSource, ModelPhase, ProducedBy, PromptTemplateName},
    graph::EntryView,
    ids::{MemoryId, Seq, TurnId},
    model::{GenerateRequest, ModelClient},
    settings::CaptureLevel,
};

use crate::agent::turn::{Recording, collect_written_memories};

/// Run one link-cleanup sweep. Returns `(new_cursor, memories_considered)`.
pub async fn catch_up(
    engine: &Engine,
    model: &dyn ModelClient,
    cursor: Seq,
) -> Result<(Seq, usize), InstanceError> {
    let head = engine.store.lock().head()?;
    if head <= cursor {
        return Ok((cursor, 0));
    }

    let Some(template) = templates::latest_template(
        engine.store.lock().as_ref(),
        PromptTemplateName::EntryConsolidation,
    )?
    else {
        return Ok((head, 0));
    };

    let written = collect_written_memories(engine.store.lock().as_ref(), cursor)?;
    if written.is_empty() {
        return Ok((head, 0));
    }

    let recording = Recording::new(None, TurnId::generate(), CaptureLevel::Off);
    let now = engine.clock.now();
    let mut events = Vec::new();

    for &id in &written {
        let (entries, links) = {
            let graph = engine.graph.lock();
            let entries = graph.class_entries(id)?;
            let links = graph.class_links(id)?;
            (entries, links)
        };
        if entries.is_empty() {
            continue;
        }

        let produced_by = ProducedBy {
            model_id: model.model_id().into(),
            template_name: PromptTemplateName::EntryConsolidation,
            template_version: template.version,
        };

        match identify_redundant(
            engine,
            model,
            &recording,
            &template.body,
            id,
            &entries,
            &links,
        )
        .await
        {
            Ok(Some(redundant_ids)) => {
                for entry_id in redundant_ids {
                    events.push(EventPayload::entry_retracted(
                        id,
                        entry_id,
                        "content fully captured by existing link",
                        Some(produced_by.clone()),
                    ));
                }
            }
            Ok(None) => {
                tracing::debug!(
                    memory = ?id,
                    "link cleanup: model returned no redundant entries"
                );
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

    if !events.is_empty() {
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

/// The model's response: the set of entry ids to retract.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct RedundantEntries {
    #[serde(default)]
    retract_entry_ids: Vec<String>,
}

/// Call the model to identify entries whose content is purely a description of a link.
async fn identify_redundant(
    engine: &Engine,
    model: &dyn ModelClient,
    recording: &Recording,
    template_body: &str,
    memory_id: MemoryId,
    entries: &[EntryView],
    links: &[crate::graph::ClassLinkView],
) -> Result<Option<Vec<crate::ids::EntryId>>, InstanceError> {
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
        "Memory: {}\n\nEntries:\n{}\n\nExisting links:\n{}\n\n\
         Mark an entry for removal only if its content is purely a description of a link that \
         exists — no additional detail (no when, no context, no qualifier). Preserve entries \
         that carry detail beyond the link.",
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
        return Ok(None);
    };

    // Parse the entry ids from strings back to EntryId.
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

    Ok(Some(entry_ids))
}
