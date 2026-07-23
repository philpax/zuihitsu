//! The canonical-profile pass: gives platform stubs readable named identities.
//!
//! For each platform stub identified since the cursor:
//! 1. If a bare (non-platform-qualified) `same_as` member is already the designated primary, skip —
//!    the stub already has a canonical identity.
//! 2. If a bare member exists but none is designated, designate that member primary and skip minting.
//!    This is the hand-merged case: the operator linked the person's `person/<name>` profile to the
//!    stub with `same_as` but never wrote a designation, so the pass completes the identity rather
//!    than colliding on the name and minting a suffixed duplicate.
//! 3. Otherwise read the stub's entries and call the model to identify a canonical name, abstaining
//!    when the evidence is weak. On a name, mint a fresh `person/<name>` profile — disambiguating with
//!    a suffix on a genuine collision with a *different* person — assert `same_as`, and designate it
//!    primary.
//!
//! Every write runs through the ordinary [`MemoryBlock`] path under [`Authority::Agent`], so the
//! profile mint, the `same_as`, and the designation clear the same guards a turn's writes do. The
//! free-merge rule that lets an agent bind a freshly-minted empty profile without an operator merge
//! proposal lives in that guard, not here.

use std::{collections::BTreeSet, sync::Arc};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    InstanceError,
    agent::templates,
    engine::Engine,
    event::{EventSource, ModelPhase, PromptTemplateName, Teller},
    graph::EntryView,
    ids::{MemoryId, MemoryName, Namespace, Seq, TurnId},
    memory::memory_block::{Authority, LinkOptions, MemoryBlock, VisibilityChoice},
    model::{GenerateRequest, ModelClient},
    settings::CaptureLevel,
    vocabulary::RelationName,
};

use crate::agent::turn::Recording;

/// Run one canonicalize sweep. Returns `(new_cursor, stubs_considered)`.
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
        PromptTemplateName::NameIdentification,
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
    let max_entry_chars = crate::settings::Settings::from_store(engine.store.lock().as_ref())
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
    // Names minted earlier in this same sweep, so two stubs identified to the same name in one window
    // do not both mint `person/<name>` (the second would fail the unique-name constraint at
    // materialize, poisoning replay); the committed graph does not yet reflect an in-sweep mint.
    let mut claimed: BTreeSet<MemoryName> = BTreeSet::new();

    for &stub_id in &stubs {
        match canonical_profile_state(engine, stub_id)? {
            CanonicalState::Designated => continue,
            CanonicalState::Undesignated(member) => {
                // The hand-merged case: a bare `same_as` member exists but no designation was ever
                // written. Designate that member primary rather than minting a colliding duplicate.
                if let Err(error) = block.designate_primary(member, true) {
                    tracing::warn!(
                        stub = ?stub_id,
                        %error,
                        "canonicalize: designation of an existing bare member rejected; skipping"
                    );
                } else {
                    tracing::info!(
                        stub = ?stub_id,
                        member = ?member,
                        "canonicalize: designated an existing bare profile primary"
                    );
                }
                continue;
            }
            CanonicalState::None => {}
        }

        // Read the stub's entries — the name evidence.
        let entries: Vec<EntryView> = {
            let graph = engine.graph.lock();
            graph.class_entries(stub_id)?
        };
        if entries.is_empty() {
            // No evidence to name from: abstain rather than guess.
            continue;
        }

        match identify_name(engine, model, &recording, &template.body, stub_id, &entries).await {
            Ok(Some(name)) => {
                let canonical_name = resolve_unique_name(engine, &name, &claimed)?;
                let handle: MemoryName = Namespace::Person.with_name(&canonical_name).into();
                match mint_canonical(&mut block, stub_id, handle.clone()) {
                    Ok(()) => {
                        claimed.insert(handle);
                        tracing::info!(
                            stub = ?stub_id,
                            canonical = %canonical_name,
                            "canonicalize: minted canonical profile"
                        );
                    }
                    Err(error) => tracing::warn!(
                        stub = ?stub_id,
                        %error,
                        "canonicalize: minting the canonical profile was rejected; skipping"
                    ),
                }
            }
            Ok(None) => {
                tracing::debug!(
                    stub = ?stub_id,
                    "canonicalize: model abstained on the name; skipping"
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

    Ok((head, stubs.len()))
}

/// Mint the canonical profile for `stub_id`: a bare empty `person/<name>` memory, bound to the stub by
/// `same_as`, and designated its class's primary. The `same_as` asserts directly (not as a merge
/// proposal) because the profile is freshly minted and empty — the free-merge case the
/// [`MemoryBlock`] guard clears under [`Authority::Agent`].
fn mint_canonical(
    block: &mut MemoryBlock,
    stub_id: MemoryId,
    handle: MemoryName,
) -> Result<(), crate::memory::memory_block::MemoryError> {
    let canonical_id = block.create(handle, None)?;
    block.link(
        stub_id,
        canonical_id,
        RelationName::SameAs,
        Some(LinkOptions {
            visibility: Some(VisibilityChoice::Public),
            exclude: None,
        }),
    )?;
    block.designate_primary(canonical_id, true)?;
    Ok(())
}

/// The model's name-identification response. The name is optional: the model omits it to abstain when
/// a stub's entries do not clearly evidence a name (an evidence-poor stub is left unnamed, never named
/// by guesswork).
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct NameIdentification {
    /// The best canonical name for this person, as a bare handle (e.g. "dave", not "person/dave"), or
    /// omitted to abstain when the entries do not clearly evidence one.
    #[serde(default)]
    name: Option<String>,
}

/// A stub's canonical-profile state, deciding what the pass does with it.
enum CanonicalState {
    /// A bare (non-platform-qualified) `same_as` member is already the designated primary — the stub
    /// has a canonical identity; nothing to do.
    Designated,
    /// A bare member exists but none is designated — the hand-merged case; designate this member
    /// primary rather than minting a duplicate.
    Undesignated(MemoryId),
    /// No bare member — identify a name and mint a canonical profile.
    None,
}

/// Collect platform stubs that were identified (bound to a platform) since the cursor.
fn collect_platform_stubs(
    store: &dyn crate::store::Store,
    cursor: Seq,
) -> Result<Vec<MemoryId>, InstanceError> {
    let mut seen = BTreeSet::new();
    let mut ordered = Vec::new();
    for event in store.read_from(cursor.next())? {
        if let crate::event::EventPayload::ParticipantIdentified { memory, .. } = event.payload
            && seen.insert(memory)
        {
            ordered.push(memory);
        }
    }
    Ok(ordered)
}

/// Classify `stub_id`'s canonical-profile state. A bare `same_as` member designated primary means the
/// stub is done; a bare member with no designation is the hand-merged case (designate it, never mint);
/// no bare member at all means the pass should identify a name and mint one. Among undesignated bare
/// members the earliest by ULID is chosen, for a deterministic outcome.
fn canonical_profile_state(
    engine: &Engine,
    stub_id: MemoryId,
) -> Result<CanonicalState, InstanceError> {
    let graph = engine.graph.lock();
    let members = graph.class_members(stub_id)?;
    let mut earliest_bare: Option<MemoryId> = None;
    for member in &members {
        let Some(memory) = graph.memory_by_id(*member)? else {
            continue;
        };
        // A canonical profile is a bare (non-platform-qualified) person name; the stub itself is
        // platform-qualified, so it is never mistaken for its own canonical profile.
        if memory.name.is_platform_qualified() {
            continue;
        }
        if graph.is_primary_designated(*member)? {
            return Ok(CanonicalState::Designated);
        }
        earliest_bare = Some(match earliest_bare {
            Some(current) => current.min(*member),
            None => *member,
        });
    }
    Ok(match earliest_bare {
        Some(member) => CanonicalState::Undesignated(member),
        None => CanonicalState::None,
    })
}

/// Resolve a name to a unique handle, appending a suffix if `person/<name>` already exists — a genuine
/// collision with a *different* person still gets a clean handle (the profiles can be merged later),
/// rather than being folded onto the existing memory. A name already claimed earlier in this sweep is
/// treated as taken too, since an in-sweep mint is not yet in the committed graph.
fn resolve_unique_name(
    engine: &Engine,
    name: &str,
    claimed: &BTreeSet<MemoryName>,
) -> Result<String, InstanceError> {
    let graph = engine.graph.lock();
    let taken = |handle: &MemoryName| -> Result<bool, InstanceError> {
        Ok(claimed.contains(handle) || graph.memory_by_name(handle.clone())?.is_some())
    };
    let base: MemoryName = Namespace::Person.with_name(name).into();
    if !taken(&base)? {
        return Ok(name.to_owned());
    }
    // Disambiguate: try name-2, name-3, etc.
    for suffix in 2.. {
        let candidate = format!("{name}-{suffix}");
        let candidate_handle: MemoryName = Namespace::Person.with_name(&candidate).into();
        if !taken(&candidate_handle)? {
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

/// Call the model to identify the canonical name from a stub's entries, or `None` when it abstains.
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

    // An abstention (no name field) leaves the stub unnamed.
    let Some(name) = parsed.name else {
        return Ok(None);
    };
    // Sanitize: the name should be a bare handle, not "person/name".
    let name = name.trim();
    let name = name
        .strip_prefix("person/")
        .unwrap_or(name)
        .trim()
        .to_owned();
    if name.is_empty() {
        return Ok(None);
    }

    Ok(Some(name))
}

#[cfg(test)]
mod tests;
