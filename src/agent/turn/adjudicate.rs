//! The off-hot-path merge adjudication: weigh each proposed cross-platform merge on the two stubs'
//! independently-recorded facts and accept or refuse it (spec §Cross-platform identity → adjudicated
//! merge).
//!
//! A turn records the agent's judgment that two `person/*` stubs may be one human as an inert
//! `MergeProposed` (no `same_as`, nothing surfaces). This background pass catches those proposals up:
//! for each, it reads both stubs' *already-recorded* facts — never the conversation that prompted the
//! proposal, which is the structural defense against a participant feeding the agent matching facts to
//! engineer a merge — and asks the model whether they coincide improbably enough to be one person,
//! given the confidences at risk. It emits a `MergeAdjudicated` either way, and on acceptance authors
//! the `same_as` link (`LinkSource::Adjudicated`) that does the merging. The judge call carries no
//! conversation, so it records no `ModelCalled` telemetry; the events it emits carry their provenance.
//! Idempotent: a proposal whose stubs are already one class is skipped, and the cursor advance keeps a
//! proposal from being re-adjudicated.

use schemars::JsonSchema;
use serde::Deserialize;

use std::collections::BTreeSet;

use crate::{
    engine::Engine,
    event::{EventPayload, LinkSource, ModelPhase, ProducedBy, PromptTemplateName, Visibility},
    graph::{EntryView, MemoryView},
    ids::{MemoryId, Seq, TurnId},
    model::{
        Completion, GenerateRequest, GenerateResponse, ModelClient, ModelError, extract_json_object,
    },
    settings::CaptureLevel,
    store::Store,
    vocabulary::RelationName,
};

use super::{Recording, TurnError, templates};

/// Catch merge adjudications up to the log (spec §Cross-platform identity → adjudicated merge): weigh
/// every `MergeProposed` in `(cursor, head]` that is not already settled, emit its `MergeAdjudicated`,
/// and on acceptance author the `same_as`. Returns the head it advanced to and how many proposals it
/// considered. Gated by the `MergeAdjudication` template existing — no template, no-op — so the feature
/// is toggled by whether its prompt is registered. Idempotent: re-running from the same cursor
/// reproduces the same verdicts, and an already-merged pair is skipped.
pub async fn run_adjudicate_catch_up(
    engine: &Engine,
    model: &dyn ModelClient,
    cursor: Seq,
) -> Result<(Seq, usize), TurnError> {
    let head = engine.store.lock().head()?;
    if head <= cursor {
        return Ok((cursor, 0));
    }
    let proposals = collect_pending_proposals(engine.store.lock().as_ref(), cursor)?;
    adjudicate(
        model,
        engine,
        &proposals,
        Recording {
            conversation: None,
            turn_id: TurnId::generate(),
            capture: CaptureLevel::Off,
        },
    )
    .await?;
    Ok((head, proposals.len()))
}

/// Adjudicate each proposed pair: read both stubs' recorded facts, ask the model to weigh them, and
/// emit the verdict (and, on acceptance, the merging `same_as`). All verdicts commit in one batch. A
/// pair already in one class is skipped; a model failure on one pair is logged and leaves the proposal
/// for the operator rather than failing the rest.
async fn adjudicate(
    model: &dyn ModelClient,
    engine: &Engine,
    proposals: &[(MemoryId, MemoryId)],
    recording: Recording,
) -> Result<(), TurnError> {
    let Some(template) = templates::latest_template(
        engine.store.lock().as_ref(),
        PromptTemplateName::MergeAdjudication,
    )?
    else {
        return Ok(());
    };
    let now = engine.clock.now();
    let mut events = Vec::new();
    for &(from, to) in proposals {
        // Read both stubs and their recorded facts (each stub's whole class), with a transient lock
        // released before the judge `.await`. Both public and private entries feed the judge: it
        // reasons internally, so a private fact is safe as corroborating evidence and never leaves the
        // adjudication — the merge decision is the only output.
        let pair = {
            let graph = engine.graph.lock();
            let (Some(from_memory), Some(to_memory)) =
                (graph.memory_by_id(from)?, graph.memory_by_id(to)?)
            else {
                continue;
            };
            // Already one identity (a prior adjudication or an operator merge) — nothing to weigh.
            let from_class = graph.class_id(from)?;
            if from_class.is_some() && from_class == graph.class_id(to)? {
                continue;
            }
            (
                from_memory,
                graph.class_entries(from)?,
                to_memory,
                graph.class_entries(to)?,
            )
        };
        let (from_memory, from_entries, to_memory, to_entries) = pair;

        let verdict = match adjudicate_pair(
            model,
            engine,
            recording,
            &template.body,
            Stub {
                memory: &from_memory,
                entries: &from_entries,
            },
            Stub {
                memory: &to_memory,
                entries: &to_entries,
            },
        )
        .await
        {
            Ok(Some(verdict)) => verdict,
            Ok(None) => continue,
            Err(error) => {
                tracing::warn!(
                    from = %from_memory.name.as_str(),
                    to = %to_memory.name.as_str(),
                    %error,
                    "merge adjudication failed; leaving the proposal for the operator"
                );
                continue;
            }
        };
        let produced_by = Some(ProducedBy {
            model_id: model.model_id().into(),
            template_name: PromptTemplateName::MergeAdjudication,
            template_version: template.version,
        });
        events.push(EventPayload::MergeAdjudicated {
            from,
            to,
            accepted: verdict.accepted,
            rationale: verdict.rationale.trim().to_owned(),
            produced_by,
        });
        // On acceptance, author the `same_as` directly with `Adjudicated` provenance — the one path to a
        // merge without the operator. The agent's own `mem:link("same_as")` is still rejected at
        // `change_link`; only this pass emits an adjudicated link, on a passing verdict.
        if verdict.accepted {
            events.push(EventPayload::LinkCreated {
                from,
                to,
                relation: RelationName::SameAs,
                source: LinkSource::Adjudicated,
            });
        }
    }

    if !events.is_empty() {
        engine.store.lock().append(now, events)?;
        // Graph (written) before store (read), per the lock-ordering rule.
        let mut graph = engine.graph.lock();
        graph.materialize_from(engine.store.lock().as_ref())?;
    }
    Ok(())
}

/// Ask the model, in one schema-constrained reply, whether two stubs' recorded facts make them the same
/// person, given the confidences at risk. The facts are listed per stub, each marked public or private
/// (so the judge can weigh confidence-at-risk), and numbered for the rationale to cite. `None` means no
/// usable reply, which the caller treats as "leave the proposal for the operator".
async fn adjudicate_pair(
    model: &dyn ModelClient,
    engine: &Engine,
    recording: Recording,
    system: &str,
    from: Stub<'_>,
    to: Stub<'_>,
) -> Result<Option<AdjudicateArgs>, ModelError> {
    let prompt = format!(
        "Two stubs are proposed to be the same person.\n\n{}\n\n{}\n\nDecide whether to merge them.",
        render_stub(from),
        render_stub(to),
    );
    let request = GenerateRequest::structured::<AdjudicateArgs>(system, prompt, "adjudicate");
    const ATTEMPTS: usize = 3;
    for attempt in 1..=ATTEMPTS {
        let record = recording.request_record(&request, None);
        let GenerateResponse { completion, .. } = recording
            .generate(engine, model, &request, ModelPhase::Synthesis, record)
            .await?;
        if let Completion::Reply(content) = completion
            && let Some(verdict) = adjudicate_argument(&content)
        {
            return Ok(Some(verdict));
        }
        tracing::debug!(
            from = %from.memory.name.as_str(),
            to = %to.memory.name.as_str(),
            attempt,
            "adjudication returned no usable JSON"
        );
    }
    tracing::warn!(
        from = %from.memory.name.as_str(),
        to = %to.memory.name.as_str(),
        attempts = ATTEMPTS,
        "adjudication gave up after retries; leaving the proposal for the operator"
    );
    Ok(None)
}

/// A stub the adjudicator weighs: its memory and the recorded facts it would contribute to a merge.
#[derive(Clone, Copy)]
struct Stub<'a> {
    memory: &'a MemoryView,
    entries: &'a [EntryView],
}

/// One stub rendered for the judge: its handle and its recorded facts, numbered, each marked `public`
/// or `private` so the model can weigh how much confidence a wrong merge would expose.
fn render_stub(stub: Stub<'_>) -> String {
    let mut out = format!("{} — recorded facts:", stub.memory.name.as_str());
    if stub.entries.is_empty() {
        out.push_str("\n  (none recorded)");
        return out;
    }
    for (index, entry) in stub.entries.iter().enumerate() {
        let visibility = match entry.visibility {
            // Attributed is an ordinary secondhand fact, not a confidence — a wrong merge exposing it
            // is low-stakes, so it weighs with public here, not private.
            Visibility::Public | Visibility::Attributed => "public",
            Visibility::PrivateToTeller | Visibility::Exclude(_) => "private",
        };
        out.push_str(&format!("\n  {}. [{visibility}] {}", index + 1, entry.text));
    }
    out
}

/// The `adjudicate` reply shape; doubles as the schema sent to the model, so prompt and parser cannot
/// drift.
#[derive(Deserialize, JsonSchema)]
struct AdjudicateArgs {
    /// True only to merge the two stubs into one identity; false to refuse (leaving them distinct for an
    /// operator to decide).
    accepted: bool,
    /// One or two sentences citing the specific recorded facts that justify the decision.
    rationale: String,
}

/// Parse the structured reply, locating the JSON object first (the model fences it). A reply that is
/// missing `accepted`/`rationale` or mis-types them fails the parse and yields `None`, so the caller
/// retries rather than guessing — a malformed verdict must never default to a merge.
fn adjudicate_argument(content: &str) -> Option<AdjudicateArgs> {
    serde_json::from_str(extract_json_object(content)?).ok()
}

/// The proposed pairs in `(cursor, head]` that are not yet settled, in first-proposal order, each
/// canonicalized so `(a, b)` and `(b, a)` coalesce. A pair the same window also adjudicates is dropped,
/// so re-proposing within a window does not double-adjudicate.
fn collect_pending_proposals(
    store: &dyn Store,
    cursor: Seq,
) -> Result<Vec<(MemoryId, MemoryId)>, TurnError> {
    let mut settled = BTreeSet::new();
    let mut seen = BTreeSet::new();
    let mut ordered = Vec::new();
    for event in store.read_from(cursor.next())? {
        match event.payload {
            EventPayload::MergeAdjudicated { from, to, .. } => {
                settled.insert(canonical_pair(from, to));
            }
            EventPayload::MergeProposed { from, to } => {
                let pair = canonical_pair(from, to);
                if seen.insert(pair) {
                    ordered.push(pair);
                }
            }
            _ => {}
        }
    }
    Ok(ordered
        .into_iter()
        .filter(|pair| !settled.contains(pair))
        .collect())
}

/// Order a pair so `(a, b)` and `(b, a)` are the same key — same_as is symmetric.
fn canonical_pair(from: MemoryId, to: MemoryId) -> (MemoryId, MemoryId) {
    if from <= to { (from, to) } else { (to, from) }
}
