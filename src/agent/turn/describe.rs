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
        ArbitrationResolution, EventPayload, ModelPhase, ProducedBy, PromptTemplateName, Teller,
        Visibility,
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

/// The description-regeneration and (optional) temporal-extraction templates a synthesis pass reads,
/// with the combined system prompt precomputed once for the whole pass.
struct SynthesisTemplates {
    description: PromptTemplate,
    extraction: Option<PromptTemplate>,
    system: String,
}

/// The pass-wide context a single memory's synthesis calls share — the model, the engine, the
/// recording seam, and the combined description-and-extraction system prompt — so a per-memory call
/// passes only what varies (the memory, its entries, and the time). The focused arbitration call
/// carries its own self-contained system prompt and so ignores this `system`.
struct SynthesisCall<'a> {
    model: &'a dyn ModelClient,
    engine: &'a Engine,
    recording: Recording,
    system: &'a str,
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
    let (memory, entries, eligible, teller_names) = {
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
        // Resolve each participant teller's handle while the graph is at hand, so the synthesis
        // prompt can attribute every numbered statement — the arbitration rules turn on who holds
        // which account, which the bare text cannot show.
        let mut teller_names: BTreeMap<MemoryId, String> = BTreeMap::new();
        for entry in &entries {
            if let Teller::Participant(teller) = entry.told_by
                && let std::collections::btree_map::Entry::Vacant(slot) = teller_names.entry(teller)
                && let Some(view) = graph.memory_by_id(teller)?
            {
                slot.insert(view.name.as_str().to_owned());
            }
        }
        (memory, entries, eligible, teller_names)
    };

    // The description and arbitration are synthesized over the memory's PUBLIC entries only, so a
    // private aside never reaches the always-visible summary (spec §Write path → from Public entries
    // only). For an all-public memory this is the whole class, unchanged.
    let public_entries: Vec<EntryView> = entries
        .iter()
        .filter(|entry| entry.visibility == Visibility::Public)
        .cloned()
        .collect();
    let call = SynthesisCall {
        model,
        engine,
        recording,
        system: &templates.system,
    };
    if !public_entries.is_empty() {
        match synthesize(&call, &memory, &public_entries, &teller_names, now).await {
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

    // The pairwise-contradiction check is its own focused model call, separate from the description
    // rewrite above: bundled into one reply, the mandatory description crowds the conditional
    // arbitration out and the model omits it entirely (spec §Write path → arbitration). It runs
    // whenever two or more public entries could collide — the arbitration is over the same numbered
    // statements the description saw — and its verdict, validated by [`arbitration_event`], is what
    // emits `BeliefArbitrated`.
    if public_entries.len() >= 2 {
        match arbitrate(&call, &memory, &public_entries, &teller_names, now).await {
            Ok(arbitration) => {
                if let Some(event) = arbitration_event(
                    id,
                    &memory,
                    arbitration,
                    &public_entries,
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

    // Private entries the agent left untimed still need temporal extraction — a private reminder must
    // still become a wake-up — but must never enter the description. A focused extract-only pass
    // resolves their occurrences; its description is discarded and it is never arbitrated (a private
    // aside is not a public belief).
    if let Some(provenance) = &extraction_provenance {
        let private_untimed: Vec<EntryView> = entries
            .iter()
            .filter(|entry| {
                entry.visibility != Visibility::Public && eligible.contains_key(&entry.entry_id)
            })
            .cloned()
            .collect();
        if !private_untimed.is_empty() {
            match synthesize(&call, &memory, &private_untimed, &teller_names, now).await {
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
/// ("last Tuesday") resolve. The pairwise contradiction check is a separate focused call ([`arbitrate`]),
/// not a rider on this reply. `None` means no usable reply came back, which the caller treats as "leave
/// the memory unchanged".
async fn synthesize(
    call: &SynthesisCall<'_>,
    memory: &MemoryView,
    entries: &[EntryView],
    teller_names: &BTreeMap<MemoryId, String>,
    now: Timestamp,
) -> Result<Option<SynthesizeArgs>, ModelError> {
    let prompt = statements_prompt(memory, entries, teller_names, now);
    // Constrain the whole reply to the `SynthesizeArgs` schema (response_format) rather than forcing a
    // tool call: serving layers that grammar-constrain the response-format path leave forced tool-call
    // *arguments* unconstrained (the Gemma 4 case), so a weak tool-caller free-forms a schema-wrong
    // shape through a tool. One fixed schema, no tool-selection needed.
    let request = GenerateRequest::structured::<SynthesizeArgs>(call.system, prompt, "synthesize");
    ask_structured(call, &request, memory, "synthesis", synthesize_argument).await
}

/// Ask the model, in its own focused schema-constrained reply, which of the numbered statements assert
/// incompatible values for the same fact (spec §Write path → arbitration). This is deliberately a
/// separate call from [`synthesize`]: bundled with the mandatory description rewrite, the conditional
/// contradiction check was crowded out and the model omitted it; alone, the check is the reply's whole
/// job. The statements are the same numbered, teller-annotated list the description saw, so the returned
/// 1-based numbers key back to `entries` in [`arbitration_event`]. `Ok(None)` means no usable reply came
/// back; a returned [`ExtractedArbitration`] is validated (>= 2 competing, non-empty statement) before
/// it emits anything.
async fn arbitrate(
    call: &SynthesisCall<'_>,
    memory: &MemoryView,
    entries: &[EntryView],
    teller_names: &BTreeMap<MemoryId, String>,
    now: Timestamp,
) -> Result<Option<ExtractedArbitration>, ModelError> {
    let mut prompt = statements_prompt(memory, entries, teller_names, now);
    // The concrete per-call ask over the numbered statements: it poses the contradiction check as the
    // reply's whole job, names the two failure modes that spuriously dissolved the conflict (a neutral
    // third statement, and each value being attributed to a different person), and asks for every
    // colliding pair. The general rules live in [`ARBITRATION_SYSTEM`].
    prompt.push_str(
        "\nCheck every pair of the numbered statements above: whenever two of them assert \
         incompatible values for the same fact — two different locations, dates, employers, or the \
         like for one thing — that pair contradicts and you must record it. A third statement that \
         names no rival value (a neutral label such as the thing's own title) does not dissolve the \
         conflict between the other two, and two accounts of the same fact attributed to different \
         people still contradict. List every contradicting pair in `competing`; when no two \
         statements collide, return an empty `competing`.\n",
    );
    let request = GenerateRequest::structured::<ExtractedArbitration>(
        ARBITRATION_SYSTEM,
        prompt,
        "arbitrate",
    );
    ask_structured(call, &request, memory, "arbitration", arbitrate_argument).await
}

/// The prompt body both synthesis calls share: the memory's name, the current time (so relative
/// phrases resolve), and the numbered, teller-annotated statements. Each statement carries its
/// attribution and assertion date, so the arbitration rules — which turn on who holds which account,
/// and when — have the facts they judge by; the bracketed metadata is for the model's judgment, never
/// content to restate.
fn statements_prompt(
    memory: &MemoryView,
    entries: &[EntryView],
    teller_names: &BTreeMap<MemoryId, String>,
    now: Timestamp,
) -> String {
    let mut prompt = format!(
        "Memory: {}\nCurrent time: {}\n\nStatements:\n",
        memory.name.as_str(),
        time::format_datetime(now),
    );
    for (index, entry) in entries.iter().enumerate() {
        let teller = match entry.told_by {
            Teller::Participant(id) => teller_names
                .get(&id)
                .map(String::as_str)
                .unwrap_or("a participant"),
            Teller::Agent => "the agent",
            Teller::Bootstrap => "genesis",
        };
        prompt.push_str(&format!(
            "{}. [from {teller} · {}] {}\n",
            index + 1,
            time::format_day(entry.asserted_at),
            entry.text
        ));
    }
    prompt.push_str(
        "\nThe bracketed attribution on each statement is metadata for your judgment, not content \
         to restate.\n",
    );
    prompt
}

/// Drive one structured synthesis `request` through the shared recording seam, retrying a few times on
/// an unusable reply before giving up (this pass is off the hot path, so a couple of extra attempts is
/// cheap). `parse` decodes the reply's content; the first reply it accepts is returned, else `None`
/// after `ATTEMPTS`. Shared by [`synthesize`] and [`arbitrate`] so both retry identically. `label`
/// names the call in the diagnostics.
async fn ask_structured<T>(
    call: &SynthesisCall<'_>,
    request: &GenerateRequest,
    memory: &MemoryView,
    label: &str,
    parse: impl Fn(&str) -> Option<T>,
) -> Result<Option<T>, ModelError> {
    let &SynthesisCall {
        model,
        engine,
        recording,
        ..
    } = call;
    const ATTEMPTS: usize = 3;
    for attempt in 1..=ATTEMPTS {
        // An off-buffer structured call; its usage must not move the conversational compaction
        // trigger, so it is read and discarded here. Each attempt is its own `Base` (a fresh
        // single-message buffer), recorded under the synthesis phase.
        let record = recording.request_record(request, None);
        let GenerateResponse { completion, .. } = recording
            .generate(engine, model, request, ModelPhase::Synthesis, record)
            .await?;
        if let Completion::Reply(content) = completion
            && let Some(args) = parse(&content)
        {
            if attempt > 1 {
                tracing::debug!(memory = %memory.name.as_str(), attempt, label, "a synthesis call succeeded after a retry");
            }
            return Ok(Some(args));
        }
        tracing::debug!(
            memory = %memory.name.as_str(),
            attempt,
            label,
            "a synthesis call returned no usable JSON"
        );
    }
    tracing::warn!(
        memory = %memory.name.as_str(),
        attempts = ATTEMPTS,
        label,
        "a synthesis call gave up after retries"
    );
    Ok(None)
}

/// The self-contained system prompt for the focused [`arbitrate`] call. It carries the whole
/// contradiction rule, since the arbitration call has no genesis template of its own: it is split out
/// of the description synthesis precisely so the check is a reply's whole job rather than a rider on the
/// mandatory description rewrite, where it was crowded out and omitted.
const ARBITRATION_SYSTEM: &str = "You audit a numbered set of statements about one thing for genuine \
    contradictions. A contradiction is two or more statements that assert incompatible values for the \
    same fact — two different locations, dates, or employers for one thing. When you find one, record \
    the colliding statement numbers in `competing`, the number(s) you judge correct in `credited` \
    (leave `credited` empty when neither is yet known to be right), and a one-line reconciling note in \
    `statement`. Two accounts of the same fact attributed to different people still contradict; do not \
    treat them as compatible merely because each holds as someone's account. Only genuine \
    contradictions count — not a fact being added, refined, or updated over time. When no two \
    statements collide, return an empty `competing`.";

/// The `synthesize` argument shape (turn-end description + temporal extraction); doubles as the
/// response-format schema, so the schema sent to the model and the parser can't drift. The
/// contradiction verdict is a separate focused call ([`arbitrate`]/[`ExtractedArbitration`]), not a
/// field here.
#[derive(Deserialize, JsonSchema)]
struct SynthesizeArgs {
    /// The memory's description as plain third-person prose — no preamble, headings, or notes.
    description: String,
    /// One entry per statement that refers to a real-world time; omit statements with no temporal
    /// reference.
    #[serde(default)]
    occurrences: Vec<ExtractedOccurrence>,
}

/// One extracted occurrence: the statement it applies to (1-based, as numbered in the prompt) and
/// the time it refers to.
#[derive(Deserialize, JsonSchema)]
struct ExtractedOccurrence {
    entry: usize,
    occurred_at: ExtractedTime,
}

/// A conflict the focused [`arbitrate`] call found among the numbered statements (spec §Write path →
/// arbitration): which statements collide, which the model credits, and a one-line reconciling note. It
/// doubles as that call's response-format schema. Statement numbers are 1-based, the same numbering
/// [`ExtractedOccurrence`] keys off.
#[derive(Deserialize, JsonSchema)]
struct ExtractedArbitration {
    /// The statement numbers (1-based) that assert incompatible values for the same fact; empty when
    /// nothing collides.
    #[serde(default)]
    competing: Vec<usize>,
    /// The statement number(s) judged correct; empty when neither account is yet known to be right, so
    /// both stand.
    #[serde(default)]
    credited: Vec<usize>,
    /// A one-line note reconciling the conflict.
    #[serde(default)]
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

/// Parse the structured-output `synthesize` reply leniently: the description is salvaged even when an
/// `occurrence` is malformed, rather than discarding the whole reply on one bad field. A smaller model
/// often mis-shapes an occurrence (flattening the nested time, or inventing one for a statement with no
/// temporal reference) while getting the description right; a strict whole-struct parse would throw all
/// of that away. Malformed occurrences are skipped, not fatal; a missing or empty description is, since
/// that is the reply's whole point. The model emits the schema as a fenced JSON block, so the object is
/// located with [`extract_json_object`] before parsing.
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
    Some(SynthesizeArgs {
        description,
        occurrences,
    })
}

/// Parse the focused `arbitrate` reply leniently, field by field, rather than strict-parsing the whole
/// object: a both-stand verdict credits neither side, and a model asked to "leave `credited` empty"
/// routinely expresses that by omitting the key or emitting `null` — a strict `Vec<usize>` parse throws
/// the whole conflict away over exactly the shape this call exists to record. Every field defaults, so
/// an empty `competing` (no conflict) or a null `credited` (both stand) parses cleanly; the returned
/// arbitration is then validated by [`arbitration_event`] (>= 2 competing, a reconciling note). `None`
/// means no JSON object came back at all. The model emits the schema as a fenced JSON block, so the
/// object is located with [`extract_json_object`] before parsing.
fn arbitrate_argument(content: &str) -> Option<ExtractedArbitration> {
    let value: serde_json::Value = serde_json::from_str(extract_json_object(content)?).ok()?;
    let statements = |key: &str| {
        value
            .get(key)
            .and_then(serde_json::Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_u64().map(|number| number as usize))
                    .collect()
            })
            .unwrap_or_default()
    };
    Some(ExtractedArbitration {
        competing: statements("competing"),
        credited: statements("credited"),
        statement: value
            .get("statement")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned(),
    })
}

#[cfg(test)]
mod tests;
