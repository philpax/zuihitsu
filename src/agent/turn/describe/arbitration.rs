//! The focused arbitration call — a separate model call that checks for pairwise contradictions
//! among the numbered statements.

use std::collections::BTreeMap;

use crate::{
    event::{ArbitrationResolution, EventPayload, ProducedBy, PromptTemplateName},
    graph::{EntryView, MemoryView},
    ids::{EntryId, MemoryId},
    model::{GenerateRequest, ModelError},
    time::Timestamp,
};

use super::{
    ExtractedArbitration, SynthesisCall,
    extract::arbitrate_argument,
    synthesis::{ask_structured, statements_prompt},
};

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

/// Ask the model, in its own focused schema-constrained reply, which of the numbered statements assert
/// incompatible values for the same fact (spec §Write path → arbitration). This is deliberately a
/// separate call from [`super::synthesis::synthesize`]: bundled with the mandatory description rewrite, the conditional
/// contradiction check was crowded out and the model omitted it; alone, the check is the reply's whole
/// job. The statements are the same numbered, teller-annotated list the description saw, so the returned
/// 1-based numbers key back to `entries` in [`arbitration_event`]. `Ok(None)` means no usable reply came
/// back; a returned [`ExtractedArbitration`] is validated (>= 2 competing, non-empty statement) before
/// it emits anything.
pub(super) async fn arbitrate(
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

/// Map a flagged conflict to a `BeliefArbitrated`, or `None` if it is malformed — fewer than two
/// distinct competing entries, or no reconciling statement (spec §Write path → arbitration). Statement
/// numbers are 1-based into `entries`, which are the Public entries the description synthesizes over,
/// so arbitration records a choice between conflicting *public* assertions.
pub(super) fn arbitration_event(
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
