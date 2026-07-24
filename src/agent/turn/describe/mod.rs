//! The off-hot-path description catch-up: regenerate the descriptions of the memories a turn wrote
//! and resolve any occurrences it left untimed (spec §Write path → regenerate off the hot path, as a
//! catch-up).
//!
//! A turn commits its entries and replies without waiting on summarization; this background pass
//! catches the descriptions up to the log afterward. Each memory whose content changed in the window
//! is re-described from its Public entries, and in the same model call the occurrence time of any
//! time-bearing statement the agent left untimed is extracted (spec §Time → "in the same pass"). The
//! synthesis calls carry no conversation, so they record no `ModelCalled` telemetry, but the events
//! they emit still carry their own provenance. The pass is idempotent: re-running from the same cursor
//! reproduces the same events.

mod arbitration;
mod extract;
mod occurrences;
mod synthesis;

use crate::{
    agent::turn::{Recording, TurnError, templates},
    engine::Engine,
    event::{EventPayload, EventSource, ProducedBy, PromptTemplateName, Teller, Visibility},
    graph::EntryView,
    ids::{EntryId, MemoryId, Seq, TurnId},
    model::ModelClient,
    settings::CaptureLevel,
    time::TemporalRef,
};
use templates::PromptTemplate;

pub(super) use extract::{ExtractedArbitration, ExtractedOccurrence, SynthesizeArgs};

/// The description-regeneration and (optional) temporal-extraction templates a synthesis pass reads,
/// with the combined system prompt precomputed once for the whole pass.
pub(super) struct SynthesisTemplates {
    pub(super) description: PromptTemplate,
    pub(super) extraction: Option<PromptTemplate>,
    pub(super) system: String,
}

/// The pass-wide context a single memory's synthesis calls share — the model, the engine, the
/// recording seam, and the combined description-and-extraction system prompt — so a per-memory call
/// passes only what varies (the memory, its entries, and the time). The focused arbitration call
/// carries its own self-contained system prompt and so ignores this `system`.
pub(super) struct SynthesisCall<'a> {
    pub(super) model: &'a dyn ModelClient,
    pub(super) engine: &'a Engine,
    pub(super) recording: &'a Recording,
    pub(super) system: &'a str,
}

/// Catch descriptions up to the log off the hot path (spec §Write path → regenerate off the hot path,
/// as a catch-up): describe every stale memory — one whose content has changed since the describer
/// last considered it — regenerating its description, arbitrating its beliefs, and resolving any
/// occurrences it left untimed, then return how many memories it considered. The whole-log pass the
/// served runtime drives on a timer and tests drive explicitly. Its synthesis
/// calls carry no conversation, so they record no `ModelCalled` telemetry; the emitted events still
/// carry their `produced_by`. Idempotent: a memory already fresh is skipped, so an idle tick is cheap.
pub async fn run_describe_catch_up(
    engine: &Engine,
    model: &dyn ModelClient,
    guard: &tokio::sync::Mutex<()>,
) -> Result<usize, TurnError> {
    let stale = engine.graph.lock().stale_memories()?;
    describe_memories(engine, model, guard, &stale).await
}

/// As [`run_describe_catch_up`], but narrowed to the stale memories among `ids` — the pass a session
/// open runs over its brief's read set, so it pays only for the descriptions the brief will read and
/// leaves the rest of the backlog to the background pass (spec §Starvation bound → composing a brief
/// forces the catch-up). A stale memory not in `ids` stays stale for the background pass — no skip, no
/// redundancy.
pub async fn run_describe_catch_up_for(
    engine: &Engine,
    model: &dyn ModelClient,
    guard: &tokio::sync::Mutex<()>,
    ids: &[MemoryId],
) -> Result<usize, TurnError> {
    let stale = engine.graph.lock().stale_memories_among(ids)?;
    describe_memories(engine, model, guard, &stale).await
}

/// Describe each candidate stale memory, holding the describer guard **per memory** rather than across
/// the whole pass: acquire it, describe one memory, append its synthesis events and the
/// `DescribePassCompleted` that marks it considered, materialize, and release — so a narrow session-open
/// pass interleaves with a long background backlog instead of waiting behind it. Staleness is re-checked
/// under the guard each iteration, so two passes never redo the same memory. The guard is the async one,
/// held by design across the model `.await`; no store or graph guard is (each is taken transiently and
/// released before a suspension point). Returns how many memories it considered.
async fn describe_memories(
    engine: &Engine,
    model: &dyn ModelClient,
    guard: &tokio::sync::Mutex<()>,
    candidates: &[MemoryId],
) -> Result<usize, TurnError> {
    if candidates.is_empty() {
        return Ok(0);
    }
    let Some(templates) = load_synthesis_templates(engine)? else {
        return Ok(0);
    };
    let recording = Recording::new(None, TurnId::generate(), CaptureLevel::Off);
    let mut considered = 0;
    for &id in candidates {
        let _guard = guard.lock().await;
        let Some((content_seq, described_seq)) = engine.graph.lock().described_state(id)? else {
            continue;
        };
        if content_seq <= described_seq {
            continue;
        }
        describe_one(engine, model, &recording, &templates, id, described_seq).await?;
        considered += 1;
    }
    Ok(considered)
}

/// Load the description-regeneration template (required) and the temporal-extraction template
/// (optional — without it the pass degrades to description-only), composing the combined system
/// prompt. `None` when no description template is registered, which skips the whole pass.
fn load_synthesis_templates(engine: &Engine) -> Result<Option<SynthesisTemplates>, TurnError> {
    let Some(description) = templates::latest_template(
        engine.store.lock().as_ref(),
        PromptTemplateName::DescriptionRegen,
    )?
    else {
        return Ok(None);
    };
    let extraction = templates::latest_template(
        engine.store.lock().as_ref(),
        PromptTemplateName::TemporalExtraction,
    )?;
    let system = synthesis::compose_synthesis_system(
        &description.body,
        extraction.as_ref().map(|template| template.body.as_str()),
    );
    Ok(Some(SynthesisTemplates {
        description,
        extraction,
        system,
    }))
}

/// Describe one stale memory: regenerate its description from its public class entries, arbitrate its
/// beliefs over the wider `Public` + `Attributed` slice (so two relayed-but-conflicting accounts the
/// agent marked `Attributed` still collide), and in the same pass resolve the occurrence of any entry
/// it left untimed since
/// `described_seq` (spec §Time → "in the same pass"). The synthesis events and a `DescribePassCompleted`
/// listing this memory commit in one batch, then materialize — so the memory reads fresh and its
/// `last_described_seq` advances past its content, whether or not synthesis produced anything (a memory
/// with no public entries is still marked considered, matching the describer's advance-past-failure
/// discipline). A model failure on the memory is logged and leaves the description unchanged.
async fn describe_one(
    engine: &Engine,
    model: &dyn ModelClient,
    recording: &Recording,
    templates: &SynthesisTemplates,
    id: MemoryId,
    described_seq: Seq,
) -> Result<(), TurnError> {
    let now = engine.clock.now();
    let mut events = Vec::new();
    let mut resolved = std::collections::BTreeSet::new();
    let extraction_provenance = templates.extraction.as_ref().map(|template| ProducedBy {
        model_id: model.model_id().into(),
        template_name: PromptTemplateName::TemporalExtraction,
        template_version: template.version,
    });

    let (memory, entries, eligible, teller_names) = {
        let graph = engine.graph.lock();
        let Some(memory) = graph.memory_by_id(id)? else {
            return Ok(());
        };
        let entries = graph.class_entries(id)?;
        let eligible: std::collections::BTreeMap<EntryId, MemoryId> = graph
            .untimed_entries_since(id, described_seq)?
            .into_iter()
            .map(|entry_id| (entry_id, id))
            .collect();
        let mut teller_names: std::collections::BTreeMap<MemoryId, String> =
            std::collections::BTreeMap::new();
        // Every teller behind an entry — its founding teller and each attesting teller — so the
        // numbered statements can name the `attested by` corroborators, not only the founding source.
        let entry_tellers = entries.iter().flat_map(|entry| {
            std::iter::once(&entry.told_by).chain(
                entry
                    .attestations
                    .iter()
                    .map(|attestation| &attestation.teller),
            )
        });
        for teller in entry_tellers {
            if let Teller::Participant(teller) = teller
                && let std::collections::btree_map::Entry::Vacant(slot) =
                    teller_names.entry(*teller)
                && let Some(view) = graph.memory_by_id(*teller)?
            {
                slot.insert(view.name.as_str().to_owned());
            }
        }
        (memory, entries, eligible, teller_names)
    };

    // The live occurrences already on the memory's entries — the description mirror's authored date
    // among them — feed the current-day guard in `resolve_occurrences`, so an extracted resolution on
    // "now" cannot silently override a differently-dated sibling.
    let siblings: Vec<TemporalRef> = entries
        .iter()
        .filter_map(|entry| entry.occurred_at.clone())
        .collect();

    let public_entries: Vec<EntryView> = entries
        .iter()
        .filter(|entry| entry.visibility == Visibility::Public)
        .cloned()
        .collect();
    // Arbitration scans a wider slice than description synthesis: an `Attributed` entry surfaces to
    // any present set like a `Public` one — its exclusion from descriptions preserves the "via
    // <teller>" provenance marker, not audience safety — so two relayed-but-conflicting accounts the
    // agent marked `Attributed` must still be arbitrated. Numbering `Public` + `Attributed` in one
    // pool lets a public account and an attributed account of the same fact collide with each other,
    // which is exactly the cross-posture contradiction worth catching. The same slice feeds both
    // `arbitrate` and `arbitration_event`, so the returned 1-based numbers key back to it.
    let arbitration_entries: Vec<EntryView> = entries
        .iter()
        .filter(|entry| {
            matches!(
                entry.visibility,
                Visibility::Public | Visibility::Attributed
            )
        })
        .cloned()
        .collect();
    let call = SynthesisCall {
        model,
        engine,
        recording,
        system: &templates.system,
    };
    if !public_entries.is_empty() {
        match synthesis::synthesize(&call, &memory, &public_entries, &teller_names, now).await {
            Ok(Some(synthesis)) => {
                if !synthesis.description.trim().is_empty() {
                    events.push(EventPayload::memory_description_regenerated(
                        id,
                        synthesis.description.trim().to_owned(),
                        Some(ProducedBy {
                            model_id: model.model_id().into(),
                            template_name: PromptTemplateName::DescriptionRegen,
                            template_version: templates.description.version,
                        }),
                    ));
                }
                if let Some(provenance) = &extraction_provenance {
                    occurrences::resolve_occurrences(
                        synthesis.occurrences,
                        &occurrences::ResolveContext {
                            list: &public_entries,
                            eligible: &eligible,
                            memory: &memory,
                            now,
                            siblings: &siblings,
                        },
                        &mut resolved,
                        provenance,
                        &mut events,
                    );
                }
            }
            Ok(None) => {}
            Err(error) => tracing::warn!(
                memory = %memory.name.as_str(),
                %error,
                "turn-end synthesis failed; keeping the prior description"
            ),
        }
    }

    if arbitration_entries.len() >= 2 {
        match arbitration::arbitrate(&call, &memory, &arbitration_entries, &teller_names, now).await
        {
            Ok(arbitration) => {
                if let Some(event) = arbitration::arbitration_event(
                    id,
                    &memory,
                    arbitration,
                    &arbitration_entries,
                    model.model_id(),
                    templates.description.version,
                ) {
                    events.push(event);
                }
            }
            Err(error) => tracing::warn!(
                memory = %memory.name.as_str(),
                %error,
                "belief arbitration failed; leaving the beliefs unarbitrated"
            ),
        }
    }

    if let Some(provenance) = &extraction_provenance {
        let private_untimed: Vec<EntryView> = entries
            .iter()
            .filter(|entry| {
                entry.visibility != Visibility::Public && eligible.contains_key(&entry.entry_id)
            })
            .cloned()
            .collect();
        if !private_untimed.is_empty() {
            match synthesis::synthesize(&call, &memory, &private_untimed, &teller_names, now).await
            {
                Ok(Some(synthesis)) => occurrences::resolve_occurrences(
                    synthesis.occurrences,
                    &occurrences::ResolveContext {
                        list: &private_untimed,
                        eligible: &eligible,
                        memory: &memory,
                        now,
                        siblings: &siblings,
                    },
                    &mut resolved,
                    provenance,
                    &mut events,
                ),
                Ok(None) => {}
                Err(error) => tracing::warn!(
                    memory = %memory.name.as_str(),
                    %error,
                    "private-entry extraction failed; leaving them untimed"
                ),
            }
        }
    }

    events.push(EventPayload::describe_pass_completed(vec![id]));
    engine
        .store
        .lock()
        .append(now, EventSource::Orchestration, events)?;
    let mut graph = engine.graph.lock();
    graph.materialize_from(engine.store.lock().as_ref())?;
    Ok(())
}

#[cfg(test)]
mod tests;
