//! Shared rendering for the replay views: a one-line summary of an [`EvalStep`] (the step-journal
//! coordinate the `events` command prints and the resume drift detector names), humane time formatting
//! (a clock advance, an event's offset from the run's first event), and a compact per-event summary.

use std::collections::HashMap;

use zuihitsu::{EventPayload, MemoryId, MemoryName, TurnRole, Usage};

use crate::{
    analyze::format::trunc,
    event_render::diagnostic_summary,
    step::{EvalStep, OnMissing, StepText},
};

/// How many characters of a step's or event's text the compact summaries keep before clipping.
const SUMMARY_CLIP: usize = 60;

/// A one-line summary of a step — the coordinate `replay --mode resume --step K` takes, and the token
/// the drift detector renders when a recorded step and the current script disagree. Mirrors the
/// [`EvalStep`] variant with its load-bearing arguments, clipping long text.
pub(crate) fn summarize_step(step: &EvalStep) -> String {
    match step {
        EvalStep::Turn(turn) => format!(
            "Turn {}/{} {}: {}",
            turn.platform,
            turn.scope,
            turn.sender,
            summarize_text(&turn.text),
        ),
        EvalStep::Imprint { text } => format!("Imprint: \"{}\"", trunc(text, SUMMARY_CLIP)),
        EvalStep::Settle => "Settle".to_owned(),
        EvalStep::Advance { millis } => format!("Advance {}", humane_duration(*millis)),
        EvalStep::DescribeCatchUp => "DescribeCatchUp".to_owned(),
        EvalStep::LinkInferenceCatchUp => "LinkInferenceCatchUp".to_owned(),
        EvalStep::CheckpointSweep => "CheckpointSweep".to_owned(),
        EvalStep::SeedEvents(events) => format!("SeedEvents (×{})", events.len()),
        EvalStep::TightenCompaction {
            token_budget,
            flush_min_turns,
        } => format!("TightenCompaction budget={token_budget} flush_min_turns={flush_min_turns}"),
        EvalStep::ForceCompaction { platform, scope } => {
            format!("ForceCompaction {platform}/{scope}")
        }
        EvalStep::TuneCheckpoint {
            min_delta_chars,
            cooldown_seconds,
            flush_on_open,
        } => format!(
            "TuneCheckpoint min_delta_chars={min_delta_chars} cooldown_seconds={cooldown_seconds} \
             flush_on_open={flush_on_open}"
        ),
        EvalStep::ConfirmProposedMerge { on_missing } => {
            let on_missing = match on_missing {
                OnMissing::Skip => "skip",
                OnMissing::Fail => "fail",
            };
            format!("ConfirmProposedMerge (on_missing: {on_missing})")
        }
    }
}

/// A step's text as a compact quoted fragment. A [`StepText::WithTurnRef`] shows its template and the
/// anchor turn it references, since the resolved `[turn:<id>]` token is only known at execution.
fn summarize_text(text: &StepText) -> String {
    match text {
        StepText::Literal(literal) => format!("\"{}\"", trunc(literal, SUMMARY_CLIP)),
        StepText::WithTurnRef { template, of_turn } => format!(
            "\"{}\" (ref: \"{}\")",
            trunc(template, SUMMARY_CLIP),
            trunc(of_turn, SUMMARY_CLIP),
        ),
    }
}

/// A duration as a compact humane string — the two most significant units, e.g. `5d`, `3d 4h`,
/// `4h 30m`, `2m10s`, `10s`. Used for a clock advance and, offset-prefixed, for an event's time from
/// the run's first event.
pub(crate) fn humane_duration(millis: i64) -> String {
    let negative = millis < 0;
    let total_secs = millis.unsigned_abs() / 1_000;
    let days = total_secs / 86_400;
    let hours = (total_secs % 86_400) / 3_600;
    let minutes = (total_secs % 3_600) / 60;
    let seconds = total_secs % 60;

    let magnitude = if days > 0 {
        two_units(days, "d", hours, "h", " ")
    } else if hours > 0 {
        two_units(hours, "h", minutes, "m", " ")
    } else if minutes > 0 {
        two_units(minutes, "m", seconds, "s", "")
    } else {
        format!("{seconds}s")
    };
    if negative {
        format!("-{magnitude}")
    } else {
        magnitude
    }
}

/// An event's offset from the run's first event, as `+<humane>` (e.g. `+0s`, `+2m10s`, `+3d 4h`).
pub(crate) fn humane_offset(millis: i64) -> String {
    format!("+{}", humane_duration(millis))
}

/// The larger unit always, the smaller only when non-zero — `3d 4h` but `5d`, joined by `sep`.
fn two_units(major: u64, major_unit: &str, minor: u64, minor_unit: &str, sep: &str) -> String {
    if minor == 0 {
        format!("{major}{major_unit}")
    } else {
        format!("{major}{major_unit}{sep}{minor}{minor_unit}")
    }
}

/// A map from a memory id to its handle, built from a run's `MemoryCreated`/`MemoryRenamed` events, so
/// a `ConversationTurn`'s participant id renders as a name rather than a raw ULID.
pub(crate) type NameMap = HashMap<MemoryId, MemoryName>;

/// Build the id→handle map from a run's events, latest name winning (a rename supersedes the created
/// name).
pub(crate) fn name_map(events: &[zuihitsu::Event]) -> NameMap {
    let mut names = NameMap::new();
    for event in events {
        match &event.payload {
            EventPayload::MemoryCreated { id, name } => {
                names.insert(*id, name.clone());
            }
            EventPayload::MemoryRenamed { id, new_name, .. } => {
                names.insert(*id, new_name.clone());
            }
            _ => {}
        }
    }
    names
}

/// A compact one-line summary of an event for the `events` listing: the conversational surfaces
/// (`ConversationTurn`, `ModelCalled`, `LuaExecuted`) rendered for legibility, the diagnostic payloads
/// reusing [`diagnostic_summary`], and a trimmed-JSON fallback for everything else so no event renders
/// blank. `truncate` clips long text (`0` = full).
pub(crate) fn event_summary(payload: &EventPayload, names: &NameMap, truncate: usize) -> String {
    let clip = |text: &str| trunc(text, truncate);
    match payload {
        EventPayload::ConversationTurn {
            role,
            text,
            participant,
            ..
        } => {
            let speaker = match (role, participant) {
                (TurnRole::Participant, Some(id)) => names
                    .get(id)
                    .map(subject)
                    .unwrap_or_else(|| "participant".to_owned()),
                (TurnRole::Participant, None) => "participant".to_owned(),
                (TurnRole::Agent, _) => "agent".to_owned(),
                (TurnRole::System, _) => "system".to_owned(),
            };
            format!("{speaker}: {}", clip(text))
        }
        EventPayload::ModelCalled { phase, usage, .. } => {
            format!("{phase:?}: {}", tokens(usage))
        }
        EventPayload::LuaExecuted {
            script,
            result,
            touched,
            ..
        } => {
            let head = script.lines().next().unwrap_or("").trim();
            let touched = if touched.is_empty() {
                String::new()
            } else {
                format!(" (touched ×{})", touched.len())
            };
            let result = match result {
                Some(result) => format!(" → {}", clip(result)),
                None => String::new(),
            };
            format!("{}{touched}{result}", clip(head))
        }
        _ => diagnostic_summary(payload)
            .map(|summary| clip(&summary))
            .unwrap_or_else(|| json_fallback(payload, truncate)),
    }
}

/// The subject of a handle (`person/marcus` → `marcus`), or the whole handle when it is in no namespace.
fn subject(name: &MemoryName) -> String {
    name.as_str()
        .rsplit_once('/')
        .map(|(_, subject)| subject.to_owned())
        .unwrap_or_else(|| name.as_str().to_owned())
}

/// A model call's token usage as `<prompt> prompt → <completion> completion`, with large counts abbreviated (`9.4k`).
fn tokens(usage: &Usage) -> String {
    format!(
        "{} prompt → {} completion",
        humane_count(usage.prompt_tokens),
        humane_count(usage.completion_tokens),
    )
}

/// A token count abbreviated at the thousands (`9400` → `9.4k`), or `?` when the backend reported none.
fn humane_count(count: Option<u32>) -> String {
    match count {
        None => "?".to_owned(),
        Some(count) if count >= 1_000 => format!("{:.1}k", count as f64 / 1_000.0),
        Some(count) => count.to_string(),
    }
}

/// The payload's fields as trimmed JSON (minus the `type` tag, already shown), for a payload with no
/// dedicated summary — so an uncovered event still renders something legible rather than nothing.
/// `truncate` clips the rendered JSON (`0` = full).
fn json_fallback(payload: &EventPayload, truncate: usize) -> String {
    match serde_json::to_value(payload) {
        Ok(serde_json::Value::Object(mut map)) => {
            map.remove("type");
            let rendered = serde_json::to_string(&map).unwrap_or_default();
            trunc(&rendered, truncate)
        }
        _ => String::new(),
    }
}
