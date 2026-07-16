//! The synthesis call — describe a memory from its entries and extract occurrence times in one
//! schema-constrained reply.

use std::collections::BTreeMap;

use crate::{
    event::ModelPhase,
    graph::{EntryView, MemoryView},
    ids::MemoryId,
    model::{Completion, GenerateRequest, GenerateResponse, ModelError},
    time::{self, Timestamp},
};

use crate::agent::turn::describe::{SynthesisCall, SynthesizeArgs, extract::synthesize_argument};

/// The synthesis call's system prompt: the description-regeneration instructions, plus the
/// temporal-extraction instructions when that template exists, joined for the single combined call
/// (spec §Time → same pass). Each half still stamps its own events' provenance.
pub(super) fn compose_synthesis_system(
    description_body: &str,
    extraction_body: Option<&str>,
) -> String {
    match extraction_body {
        Some(extraction) => format!("{description_body}\n\n{extraction}"),
        None => description_body.to_owned(),
    }
}

/// Ask the model, in one schema-constrained `synthesize` reply, to describe a memory from its entries
/// and extract the occurrence time of any time-bearing statement. The entries are numbered (1-based) so
/// the extracted occurrences key back to them, and the current time is stated so relative phrases
/// ("last Tuesday") resolve. The pairwise contradiction check is a separate focused call ([`crate::agent::turn::describe::arbitration::arbitrate`]),
/// not a rider on this reply. `None` means no usable reply came back, which the caller treats as "leave
/// the memory unchanged".
pub(super) async fn synthesize(
    call: &SynthesisCall<'_>,
    memory: &MemoryView,
    entries: &[EntryView],
    teller_names: &BTreeMap<MemoryId, String>,
    now: Timestamp,
) -> Result<Option<SynthesizeArgs>, ModelError> {
    let prompt = statements_prompt(memory, entries, teller_names, now);
    let request = GenerateRequest::structured::<SynthesizeArgs>(call.system, prompt, "synthesize");
    ask_structured(call, &request, memory, "synthesis", synthesize_argument).await
}

/// The prompt body both synthesis calls share: the memory's name, the current time (so relative
/// phrases resolve), and the numbered, teller-annotated statements. Each statement carries its
/// attribution, assertion date, and — when it has one — its recorded occurrence, so the arbitration
/// rules (which turn on who holds which account, and when) and the temporal extraction (which anchors
/// a back-pointing phrase like "this date" against a sibling's stated occurrence) have the facts they judge by; the
/// bracketed metadata is for the model's judgment, never content to restate.
pub(super) fn statements_prompt(
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
            crate::event::Teller::Participant(id) => teller_names
                .get(&id)
                .map(String::as_str)
                .unwrap_or("a participant"),
            crate::event::Teller::Agent => "the agent",
            crate::event::Teller::Bootstrap => "genesis",
        };
        // A dated statement carries its occurrence in the bracket, so a back-pointing phrase in an undated sibling
        // ("this date") resolves against the stated date rather than the conversation's "now".
        let occurred = match &entry.occurred_at {
            Some(occurred_at) => format!(" · occurred {}", time::format_occurrence(occurred_at)),
            None => String::new(),
        };
        prompt.push_str(&format!(
            "{}. [from {teller} · {}{occurred}] {}\n",
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
/// after `ATTEMPTS`. Shared by [`synthesize`] and [`crate::agent::turn::describe::arbitration::arbitrate`] so both retry identically. `label`
/// names the call in the diagnostics.
pub(super) async fn ask_structured<T>(
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
        // The synthesis prompt is not the six-section assembled prompt, so it carries no typed
        // section spans.
        let record = recording.request_record(request, None, &[]);
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
