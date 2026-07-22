//! The canonical-profile pass: gives platform stubs readable named identities.
//!
//! For each platform stub minted since the cursor:
//! 1. Read its entries to identify a name.
//! 2. Call the model to pick the most name-like text from the stub's entries.
//! 3. Check if `person/<name>` already exists; if so, disambiguate with a suffix.
//! 4. Create the canonical profile, assert `same_as` (under `Authority::Agent`), and designate
//!    it primary.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    InstanceError,
    agent::templates,
    engine::Engine,
    event::{EventPayload, EventSource, ModelPhase, PromptTemplateName, Teller, Visibility},
    graph::EntryView,
    ids::{MemoryId, Namespace, Seq, TurnId},
    model::{GenerateRequest, ModelClient},
    settings::CaptureLevel,
};

use crate::agent::turn::Recording;

/// Run one canonicalize sweep. Returns `(new_cursor, stubs_considered)`.
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

    // Collect platform stubs identified since the cursor.
    let stubs = collect_platform_stubs(engine.store.lock().as_ref(), cursor)?;
    if stubs.is_empty() {
        return Ok((head, 0));
    }

    let recording = Recording::new(None, TurnId::generate(), CaptureLevel::Off);
    let now = engine.clock.now();
    let mut events = Vec::new();

    for &stub_id in &stubs {
        // Check if the stub already has a canonical profile (a non-platform-qualified same_as
        // member designated as primary). If so, skip.
        if has_canonical_profile(engine, stub_id)? {
            continue;
        }

        // Read the stub's entries.
        let entries: Vec<EntryView> = {
            let graph = engine.graph.lock();
            graph.class_entries(stub_id)?
        };
        if entries.is_empty() {
            continue;
        }

        // Call the model to identify the name.
        match identify_name(engine, model, &recording, &template.body, stub_id, &entries).await {
            Ok(Some(name)) => {
                let canonical_name = resolve_unique_name(engine, &name)?;
                let canonical_id = MemoryId::generate();

                // Create the canonical profile (bare, no content).
                events.push(EventPayload::memory_created(
                    canonical_id,
                    Namespace::Person.with_name(&canonical_name),
                ));
                // Assert same_as between the stub and the canonical profile.
                events.push(EventPayload::link_created(
                    stub_id,
                    canonical_id,
                    crate::vocabulary::RelationName::SameAs,
                    crate::event::LinkPosture {
                        source: crate::event::LinkSource::Agent,
                        told_by: Some(Teller::Agent),
                        told_in: None,
                        visibility: Visibility::Public,
                    },
                ));
                // Designate the canonical profile as primary.
                events.push(EventPayload::class_primary_designated(canonical_id, true));

                tracing::info!(
                    stub = ?stub_id,
                    canonical = %canonical_name,
                    "canonicalize: minted canonical profile"
                );
            }
            Ok(None) => {
                tracing::debug!(
                    stub = ?stub_id,
                    "canonicalize: model returned no name; skipping"
                );
            }
            Err(error) => {
                tracing::warn!(
                    stub = ?stub_id,
                    %error,
                    "canonicalize: name identification failed; skipping"
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

    Ok((head, stubs.len()))
}

/// The model's name-identification response.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct NameIdentification {
    /// The best canonical name for this person, as a bare handle (e.g. "dave", not "person/dave").
    name: String,
}

/// Collect platform stubs that were identified (bound to a platform) since the cursor.
fn collect_platform_stubs(
    store: &dyn crate::store::Store,
    cursor: Seq,
) -> Result<Vec<MemoryId>, InstanceError> {
    let mut seen = std::collections::BTreeSet::new();
    let mut ordered = Vec::new();
    for event in store.read_from(cursor.next())? {
        if let EventPayload::ParticipantIdentified { memory, .. } = event.payload
            && seen.insert(memory)
        {
            ordered.push(memory);
        }
    }
    Ok(ordered)
}

/// Whether `stub_id` already has a canonical profile: a non-platform-qualified `same_as` member
/// designated as primary.
fn has_canonical_profile(engine: &Engine, stub_id: MemoryId) -> Result<bool, InstanceError> {
    let graph = engine.graph.lock();
    let members = graph.class_members(stub_id)?;
    for member in &members {
        if let Some(memory) = graph.memory_by_id(*member)? {
            // A canonical profile is a bare (non-platform-qualified) person name.
            if !memory.name.is_platform_qualified() && graph.is_primary_designated(*member)? {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// Resolve a name to a unique handle, appending a suffix if `person/<name>` already exists.
fn resolve_unique_name(engine: &Engine, name: &str) -> Result<String, InstanceError> {
    let graph = engine.graph.lock();
    let base = Namespace::Person.with_name(name);
    if graph.memory_by_name(&base)?.is_none() {
        return Ok(name.to_owned());
    }
    // Disambiguate: try name-2, name-3, etc.
    for suffix in 2.. {
        let candidate = format!("{name}-{suffix}");
        let candidate_name = Namespace::Person.with_name(&candidate);
        if graph.memory_by_name(&candidate_name)?.is_none() {
            tracing::info!(
                original = name,
                disambiguated = %candidate,
                "canonicalize: name collision resolved with a disambiguating suffix"
            );
            return Ok(candidate);
        }
    }
    // Unreachable in practice (sufficient suffixes exist), but fall back to a ULID to be safe.
    Ok(format!("{name}-{}", MemoryId::generate().0))
}

/// Call the model to identify the canonical name from a stub's entries.
async fn identify_name(
    engine: &Engine,
    model: &dyn ModelClient,
    recording: &Recording,
    template_body: &str,
    stub_id: MemoryId,
    entries: &[EntryView],
) -> Result<Option<String>, InstanceError> {
    let entry_lines: Vec<String> = entries.iter().map(|e| format!("- {:?}", e.text)).collect();
    let user_prompt = format!(
        "Platform stub: {}\n\nEntries on this stub:\n{}",
        stub_id.0,
        entry_lines.join("\n")
    );

    let request = GenerateRequest::structured::<NameIdentification>(
        template_body,
        user_prompt,
        "name_identification",
    );

    let record = recording.request_record(&request, None, &[]);
    let response = recording
        .generate(engine, model, &request, ModelPhase::Synthesis, record, None)
        .await
        .map_err(|e| InstanceError::from(crate::agent::TurnError::Model(e)))?
        .expect_completed();

    let Some(parsed) = crate::model::parse_structured::<NameIdentification>(&response.completion)
    else {
        return Ok(None);
    };

    // Sanitize: the name should be a bare handle, not "person/name".
    let name = parsed.name.trim().to_owned();
    let name = name
        .strip_prefix("person/")
        .unwrap_or(&name)
        .trim()
        .to_owned();
    if name.is_empty() {
        return Ok(None);
    }

    Ok(Some(name))
}
