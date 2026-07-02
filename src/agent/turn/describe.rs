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

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use std::collections::{BTreeMap, BTreeSet};

use crate::{
    engine::Engine,
    event::{
        ArbitrationResolution, EventPayload, ModelPhase, ProducedBy, PromptTemplateName, Visibility,
    },
    graph::{EntryView, MemoryView},
    ids::{EntryId, MemoryId, MemoryName, Seq, TurnId},
    model::{
        Completion, GenerateRequest, GenerateResponse, ModelClient, ModelError, extract_json_object,
    },
    settings::CaptureLevel,
    time::{self, CivilDate, Direction, Rrule, TemporalRef, Timestamp},
};

use super::{Recording, TurnError, templates};
use templates::PromptTemplate;

/// Catch descriptions up to the log off the hot path (spec §Write path → regenerate off the hot path,
/// as a catch-up): describe every stale memory — one whose content has changed since the describer
/// last considered it — regenerating its description, arbitrating its beliefs, and resolving any
/// occurrences it left untimed, then return how many memories it considered. The whole-log pass the
/// served runtime drives on a timer and tests and the eval harness drive explicitly. Its synthesis
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

/// The description-regeneration and (optional) temporal-extraction templates a synthesis pass reads,
/// with the combined system prompt precomputed once for the whole pass.
struct SynthesisTemplates {
    description: PromptTemplate,
    extraction: Option<PromptTemplate>,
    system: String,
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
    let recording = Recording {
        conversation: None,
        turn_id: TurnId::generate(),
        capture: CaptureLevel::Off,
    };
    let mut considered = 0;
    for &id in candidates {
        // Held across this one memory's synthesis so a concurrent pass waits, then re-reads the
        // advanced watermark and skips it. Released at the end of the iteration.
        let _guard = guard.lock().await;
        let Some((content_seq, described_seq)) = engine.graph.lock().described_state(id)? else {
            // Unknown or soft-deleted since the candidate set was taken — nothing to describe.
            continue;
        };
        if content_seq <= described_seq {
            // A concurrent pass already caught this memory up.
            continue;
        }
        describe_one(engine, model, recording, &templates, id, described_seq).await?;
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
    let system = compose_synthesis_system(
        &description.body,
        extraction.as_ref().map(|template| template.body.as_str()),
    );
    Ok(Some(SynthesisTemplates {
        description,
        extraction,
        system,
    }))
}

/// Describe one stale memory: regenerate its description and arbitrate its beliefs from its public
/// class entries, and in the same pass resolve the occurrence of any entry it left untimed since
/// `described_seq` (spec §Time → "in the same pass"). The synthesis events and a `DescribePassCompleted`
/// listing this memory commit in one batch, then materialize — so the memory reads fresh and its
/// `last_described_seq` advances past its content, whether or not synthesis produced anything (a memory
/// with no public entries is still marked considered, matching the describer's advance-past-failure
/// discipline). A model failure on the memory is logged and leaves the description unchanged.
async fn describe_one(
    engine: &Engine,
    model: &dyn ModelClient,
    recording: Recording,
    templates: &SynthesisTemplates,
    id: MemoryId,
    described_seq: Seq,
) -> Result<(), TurnError> {
    let now = engine.clock.now();
    let mut events = Vec::new();
    let mut resolved = BTreeSet::new();
    let extraction_provenance = templates.extraction.as_ref().map(|template| ProducedBy {
        model_id: model.model_id().into(),
        template_name: PromptTemplateName::TemporalExtraction,
        template_version: template.version,
    });

    // Read the memory, its whole same_as class, and the entries it left untimed since it was last
    // described, with a transient lock released before the synthesis `.await` — no graph guard is held
    // across a suspension point. A class-wide read gives a merged identity one unified description
    // (spec §Visibility); the untimed window filters to this memory's own entries.
    let (memory, entries, eligible) = {
        let graph = engine.graph.lock();
        let Some(memory) = graph.memory_by_id(id)? else {
            return Ok(());
        };
        let entries = graph.class_entries(id)?;
        let eligible: BTreeMap<EntryId, MemoryId> = graph
            .untimed_entries_since(id, described_seq)?
            .into_iter()
            .map(|entry_id| (entry_id, id))
            .collect();
        (memory, entries, eligible)
    };

    // The description and arbitration are synthesized over the memory's PUBLIC entries only, so a
    // private aside never reaches the always-visible summary (spec §Write path → from Public entries
    // only). For an all-public memory this is the whole class, unchanged.
    let public_entries: Vec<EntryView> = entries
        .iter()
        .filter(|entry| entry.visibility == Visibility::Public)
        .cloned()
        .collect();
    if !public_entries.is_empty() {
        match synthesize(
            model,
            engine,
            recording,
            &templates.system,
            &memory,
            &public_entries,
            now,
        )
        .await
        {
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
                if let Some(event) = arbitration_event(
                    id,
                    &memory,
                    synthesis.arbitration,
                    &public_entries,
                    model.model_id(),
                    templates.description.version,
                ) {
                    events.push(event);
                }
                if let Some(provenance) = &extraction_provenance {
                    resolve_occurrences(
                        synthesis.occurrences,
                        &public_entries,
                        &eligible,
                        &mut resolved,
                        provenance,
                        &memory,
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

    // Private entries the agent left untimed still need temporal extraction — a private reminder must
    // still become a wake-up — but must never enter the description. A focused extract-only pass
    // resolves their occurrences; its description and arbitration are discarded.
    if let Some(provenance) = &extraction_provenance {
        let private_untimed: Vec<EntryView> = entries
            .iter()
            .filter(|entry| {
                entry.visibility != Visibility::Public && eligible.contains_key(&entry.entry_id)
            })
            .cloned()
            .collect();
        if !private_untimed.is_empty() {
            match synthesize(
                model,
                engine,
                recording,
                &templates.system,
                &memory,
                &private_untimed,
                now,
            )
            .await
            {
                Ok(Some(synthesis)) => resolve_occurrences(
                    synthesis.occurrences,
                    &private_untimed,
                    &eligible,
                    &mut resolved,
                    provenance,
                    &memory,
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

    // Always record the pass over this memory, even when synthesis produced nothing (an empty or
    // all-private memory still counts as considered), so its `last_described_seq` advances and it does
    // not churn back into the stale set on the next tick.
    events.push(EventPayload::describe_pass_completed(vec![id]));
    engine.store.lock().append(now, events)?;
    // Two guards at once: graph (written) before store (read), per the lock-ordering rule.
    let mut graph = engine.graph.lock();
    graph.materialize_from(engine.store.lock().as_ref())?;
    Ok(())
}

/// Map a flagged conflict to a `BeliefArbitrated`, or `None` if it is malformed — fewer than two
/// distinct competing entries, or no reconciling statement (spec §Write path → arbitration). Statement
/// numbers are 1-based into `entries`, which are the Public entries the description synthesizes over,
/// so arbitration records a choice between conflicting *public* assertions.
fn arbitration_event(
    memory_id: MemoryId,
    memory: &MemoryView,
    arbitration: Option<ExtractedArbitration>,
    entries: &[EntryView],
    model_id: &str,
    template_version: u32,
) -> Option<EventPayload> {
    let arbitration = arbitration?;
    let to_entry_ids = |numbers: Vec<usize>| {
        let mut ids: Vec<EntryId> = Vec::new();
        for number in numbers {
            if let Some(entry) = number.checked_sub(1).and_then(|i| entries.get(i))
                && !ids.contains(&entry.entry_id)
            {
                ids.push(entry.entry_id);
            }
        }
        ids
    };
    let competing_entries = to_entry_ids(arbitration.competing);
    let credited = to_entry_ids(arbitration.credited);
    if competing_entries.len() < 2 || arbitration.statement.trim().is_empty() {
        tracing::debug!(memory = %memory.name.as_str(), "dropping a malformed arbitration");
        return None;
    }
    Some(EventPayload::belief_arbitrated(
        memory_id,
        competing_entries,
        ArbitrationResolution {
            credited,
            statement: arbitration.statement.trim().to_owned(),
        },
        Some(ProducedBy {
            model_id: model_id.into(),
            template_name: PromptTemplateName::DescriptionRegen,
            template_version,
        }),
    ))
}

/// Resolve the extracted `occurrences` for the entries `list` (1-based statement numbers), pushing an
/// `EntryTemporalResolved` for each new, untimed entry, once. Shared by the public synthesis pass and
/// the focused private-entry extraction pass, so each only resolves the entries it was shown.
fn resolve_occurrences(
    occurrences: Vec<ExtractedOccurrence>,
    list: &[EntryView],
    eligible: &BTreeMap<EntryId, MemoryId>,
    resolved: &mut BTreeSet<EntryId>,
    provenance: &ProducedBy,
    memory: &MemoryView,
    events: &mut Vec<EventPayload>,
) {
    for occurrence in occurrences {
        // The statement number is 1-based into the entries listed in the prompt.
        let Some(entry) = occurrence.entry.checked_sub(1).and_then(|i| list.get(i)) else {
            continue;
        };
        // Only a new, untimed entry; skip anything else the model keyed (an entry already timed,
        // explicitly set, or a class sibling not written this turn), and resolve each once.
        let Some(&entry_memory) = eligible.get(&entry.entry_id) else {
            continue;
        };
        if !resolved.insert(entry.entry_id) {
            continue;
        }
        let raw_occurred_at = occurrence.occurred_at.clone();
        let occurred_at = match occurrence.occurred_at.into_temporal_ref() {
            Some(occurred_at) => occurred_at,
            None => {
                let raw = serde_json::to_string(&raw_occurred_at).unwrap_or_default();
                tracing::warn!(
                    memory = %memory.name.as_str(),
                    %raw,
                    "dropping an unparseable extracted occurrence; the model emitted a temporal reference this build cannot interpret"
                );
                events.push(EventPayload::entry_temporal_resolve_failed(
                    entry_memory,
                    entry.entry_id,
                    raw,
                    "unparseable temporal reference".to_owned(),
                    Some(provenance.clone()),
                ));
                continue;
            }
        };
        events.push(EventPayload::entry_temporal_resolved(
            entry_memory,
            entry.entry_id,
            occurred_at,
            Some(provenance.clone()),
        ));
    }
}

/// The synthesis call's system prompt: the description-regeneration instructions, plus the
/// temporal-extraction instructions when that template exists, joined for the single combined call
/// (spec §Time → same pass). Each half still stamps its own events' provenance.
fn compose_synthesis_system(description_body: &str, extraction_body: Option<&str>) -> String {
    match extraction_body {
        Some(extraction) => format!("{description_body}\n\n{extraction}"),
        None => description_body.to_owned(),
    }
}

/// Ask the model, in one schema-constrained `synthesize` reply, to describe a memory from its entries
/// and extract the occurrence time of any time-bearing statement. The entries are numbered (1-based) so
/// the extracted occurrences key back to them, and the current time is stated so relative phrases
/// ("last Tuesday") resolve. `None` means no usable reply came back, which the caller treats as "leave
/// the memory unchanged".
async fn synthesize(
    model: &dyn ModelClient,
    engine: &Engine,
    recording: Recording,
    system: &str,
    memory: &MemoryView,
    entries: &[EntryView],
    now: Timestamp,
) -> Result<Option<SynthesizeArgs>, ModelError> {
    let mut prompt = format!(
        "Memory: {}\nCurrent time: {}\n\nStatements:\n",
        memory.name.as_str(),
        time::format_datetime(now),
    );
    for (index, entry) in entries.iter().enumerate() {
        prompt.push_str(&format!("{}. {}\n", index + 1, entry.text));
    }

    // Constrain the whole reply to the `SynthesizeArgs` schema (response_format) rather than forcing a
    // tool call: serving layers that grammar-constrain the response-format path leave forced tool-call
    // *arguments* unconstrained (the Gemma 4 case), so a weak tool-caller free-forms a schema-wrong
    // shape through a tool. One fixed schema, no tool-selection needed.
    let request = GenerateRequest::structured::<SynthesizeArgs>(system, prompt, "synthesize");
    // The model can still emit unusable JSON (or wrap it oddly); retry a few times before giving up
    // (this pass is off the hot path, so a couple of extra attempts is cheap).
    const ATTEMPTS: usize = 3;
    for attempt in 1..=ATTEMPTS {
        // An off-buffer structured call; its usage must not move the conversational compaction
        // trigger, so it is read and discarded here. Each attempt is its own `Base` (a fresh
        // single-message buffer), recorded under the synthesis phase.
        let record = recording.request_record(&request, None);
        let GenerateResponse { completion, .. } = recording
            .generate(engine, model, &request, ModelPhase::Synthesis, record)
            .await?;
        if let Completion::Reply(content) = completion
            && let Some(args) = synthesize_argument(&content)
        {
            if attempt > 1 {
                tracing::debug!(memory = %memory.name.as_str(), attempt, "synthesis succeeded after a retry");
            }
            return Ok(Some(args));
        }
        tracing::debug!(
            memory = %memory.name.as_str(),
            attempt,
            "synthesis returned no usable JSON"
        );
    }
    tracing::warn!(
        memory = %memory.name.as_str(),
        attempts = ATTEMPTS,
        "synthesis gave up after retries; keeping the memory unchanged"
    );
    Ok(None)
}

/// The `synthesize` argument shape (turn-end description + temporal extraction); doubles as the
/// tool's parameter schema, so the schema sent to the model and the parser can't drift.
#[derive(Deserialize, JsonSchema)]
struct SynthesizeArgs {
    /// The memory's description as plain third-person prose — no preamble, headings, or notes.
    description: String,
    /// One entry per statement that refers to a real-world time; omit statements with no temporal
    /// reference.
    #[serde(default)]
    occurrences: Vec<ExtractedOccurrence>,
    /// Present only when two or more statements directly contradict each other; absent otherwise.
    #[serde(default)]
    arbitration: Option<ExtractedArbitration>,
}

/// One extracted occurrence: the statement it applies to (1-based, as numbered in the prompt) and
/// the time it refers to.
#[derive(Deserialize, JsonSchema)]
struct ExtractedOccurrence {
    entry: usize,
    occurred_at: ExtractedTime,
}

/// A conflict the synthesis found among the numbered statements (spec §Write path → arbitration):
/// which statements collide, which the model credits, and a one-line reconciling note. Statement
/// numbers are 1-based, the same numbering [`ExtractedOccurrence`] keys off.
#[derive(Deserialize, JsonSchema)]
struct ExtractedArbitration {
    competing: Vec<usize>,
    credited: Vec<usize>,
    statement: String,
}

/// The date-string occurrence shape the model produces — it cannot compute epoch milliseconds, so it
/// emits ISO dates (and occasionally datetimes), which [`ExtractedTime::into_temporal_ref`] maps to
/// the stored [`TemporalRef`]. Mirrors `TemporalRef`'s tags but with string dates.
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum ExtractedTime {
    Instant(String),
    Day(String),
    Range {
        start: String,
        end: String,
    },
    Approx {
        center: String,
        fuzz_days: u32,
    },
    /// An RFC 5545 recurrence rule, e.g. `FREQ=WEEKLY;BYDAY=MO`. Only `FREQ` and `INTERVAL` are
    /// interpreted; bare English cadences like "every Monday" are dropped.
    Recurring(String),
    BeforeAfter {
        dir: String,
        anchor: String,
    },
}

impl ExtractedTime {
    /// Map the model's date strings to the stored [`TemporalRef`], or `None` if a date won't parse.
    /// A bare calendar day under `instant` becomes a `Day`: a live probe showed the model uses the
    /// two interchangeably.
    fn into_temporal_ref(self) -> Option<TemporalRef> {
        match self {
            ExtractedTime::Instant(text) => match civil_date(&text) {
                Some(day) => Some(TemporalRef::Day(day)),
                None => Some(TemporalRef::Instant(Timestamp::from_millis(
                    time::datetime_to_millis(&text)?,
                ))),
            },
            ExtractedTime::Day(text) => civil_date(&text).map(TemporalRef::Day),
            ExtractedTime::Range { start, end } => Some(TemporalRef::Range {
                start: Timestamp::from_millis(time::date_or_datetime_to_millis(&start)?),
                end: Timestamp::from_millis(time::date_or_datetime_to_millis(&end)?),
            }),
            ExtractedTime::Approx { center, fuzz_days } => Some(TemporalRef::Approx {
                center: Timestamp::from_millis(time::date_or_datetime_to_millis(&center)?),
                fuzz_days,
            }),
            ExtractedTime::Recurring(rule) => {
                // Reject a rule this build cannot interpret (a model free-phrasing such as "every
                // Monday") rather than committing a Recurring entry that parses to no occurrence and
                // so silently never fires. Treated as unparseable, so resolve_occurrences drops it.
                let rule = Rrule(rule.into());
                time::rrule_is_supported(&rule).then_some(TemporalRef::Recurring(rule))
            }
            ExtractedTime::BeforeAfter { dir, anchor } => {
                let dir = match dir.trim().to_ascii_lowercase().as_str() {
                    "before" => Direction::Before,
                    "after" => Direction::After,
                    _ => return None,
                };
                Some(TemporalRef::BeforeAfter {
                    dir,
                    anchor: MemoryName::new(anchor),
                })
            }
        }
    }
}

/// The model's date string as a validated `Day` civil date, or `None`. A bare `YYYY-MM-DD` under
/// `instant` becomes a `Day` (the model uses the two interchangeably).
fn civil_date(text: &str) -> Option<CivilDate> {
    let date = CivilDate(text.trim().into());
    date.midnight_millis().map(|_| date)
}

/// Parse the structured-output `synthesize` reply leniently: the description and any arbitration are
/// salvaged even when an `occurrence` is malformed, rather than discarding the whole reply on one bad
/// field. A smaller model often mis-shapes an occurrence (flattening the nested time, or inventing one
/// for a statement with no temporal reference) while getting the description and arbitration right; a
/// strict whole-struct parse would throw all of that away. Malformed occurrences are skipped, not
/// fatal; a missing or empty description is, since that is the reply's whole point. The model emits the
/// schema as a fenced JSON block, so the object is located with [`extract_json_object`] before parsing.
fn synthesize_argument(content: &str) -> Option<SynthesizeArgs> {
    let value: serde_json::Value = serde_json::from_str(extract_json_object(content)?).ok()?;

    let description = value.get("description")?.as_str()?.trim().to_owned();
    if description.is_empty() {
        return None;
    }
    let occurrences = value
        .get("occurrences")
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| serde_json::from_value::<ExtractedOccurrence>(item.clone()).ok())
                .collect()
        })
        .unwrap_or_default();
    let arbitration = value
        .get("arbitration")
        .and_then(|value| serde_json::from_value::<ExtractedArbitration>(value.clone()).ok());
    Some(SynthesizeArgs {
        description,
        occurrences,
        arbitration,
    })
}

#[cfg(test)]
mod tests {
    use super::ExtractedTime;
    use crate::{
        ids::MemoryName,
        time::{self, CivilDate, Rrule, TemporalRef, Timestamp},
    };

    fn ms(date: &str) -> i64 {
        time::civil_date_to_millis(date).unwrap()
    }

    #[test]
    fn instant_date_only_coerces_to_day() {
        // The model uses `instant` for bare days; a date-only value becomes a `Day`, not an `Instant`.
        assert_eq!(
            ExtractedTime::Instant("2026-06-03".to_owned()).into_temporal_ref(),
            Some(TemporalRef::Day(CivilDate("2026-06-03".into())))
        );
    }

    #[test]
    fn instant_with_a_time_stays_an_instant() {
        let at = time::datetime_to_millis("2026-06-02T09:30:00Z").unwrap();
        assert_eq!(
            ExtractedTime::Instant("2026-06-02T09:30:00Z".to_owned()).into_temporal_ref(),
            Some(TemporalRef::Instant(Timestamp::from_millis(at)))
        );
    }

    #[test]
    fn day_maps_through() {
        assert_eq!(
            ExtractedTime::Day("2026-06-03".to_owned()).into_temporal_ref(),
            Some(TemporalRef::Day(CivilDate("2026-06-03".into())))
        );
    }

    #[test]
    fn range_and_approx_convert_dates_to_millis() {
        assert_eq!(
            ExtractedTime::Range {
                start: "2019-01-01".to_owned(),
                end: "2019-12-31".to_owned(),
            }
            .into_temporal_ref(),
            Some(TemporalRef::Range {
                start: Timestamp::from_millis(ms("2019-01-01")),
                end: Timestamp::from_millis(ms("2019-12-31")),
            })
        );
        assert_eq!(
            ExtractedTime::Approx {
                center: "2024-06-07".to_owned(),
                fuzz_days: 60,
            }
            .into_temporal_ref(),
            Some(TemporalRef::Approx {
                center: Timestamp::from_millis(ms("2024-06-07")),
                fuzz_days: 60,
            })
        );
    }

    #[test]
    fn before_after_parses_direction_case_insensitively() {
        assert_eq!(
            ExtractedTime::BeforeAfter {
                dir: "After".to_owned(),
                anchor: "event/wedding".to_owned(),
            }
            .into_temporal_ref(),
            Some(TemporalRef::after(MemoryName::new("event/wedding")))
        );
        // An unrecognized direction drops the occurrence rather than guessing.
        assert_eq!(
            ExtractedTime::BeforeAfter {
                dir: "sideways".to_owned(),
                anchor: "x".to_owned(),
            }
            .into_temporal_ref(),
            None
        );
    }

    #[test]
    fn malformed_dates_drop() {
        // 2026 is not a leap year, so Feb 29 is impossible; a non-date instant has no datetime either.
        assert_eq!(
            ExtractedTime::Day("2026-02-29".to_owned()).into_temporal_ref(),
            None
        );
        assert_eq!(
            ExtractedTime::Instant("whenever".to_owned()).into_temporal_ref(),
            None
        );
        assert_eq!(
            ExtractedTime::Range {
                start: "nope".to_owned(),
                end: "2020-01-01".to_owned(),
            }
            .into_temporal_ref(),
            None
        );
    }

    #[test]
    fn a_supported_recurrence_is_kept_and_a_free_phrase_is_dropped() {
        // A well-formed rule arms a wake-up, so it is committed.
        assert_eq!(
            ExtractedTime::Recurring("FREQ=WEEKLY;BYDAY=MO".to_owned()).into_temporal_ref(),
            Some(TemporalRef::Recurring(Rrule("FREQ=WEEKLY;BYDAY=MO".into())))
        );
        // A free-phrased cadence ("every Monday") is not an rrule this build interprets: dropping it
        // here leaves the entry untimed, rather than committing a Recurring that silently never fires.
        assert_eq!(
            ExtractedTime::Recurring("every Monday".to_owned()).into_temporal_ref(),
            None
        );
        assert_eq!(
            ExtractedTime::Recurring("FREQ=HOURLY".to_owned()).into_temporal_ref(),
            None
        );
    }
}
