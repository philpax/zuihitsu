//! Shared event-rendering primitives: the payload's type name and the compact one-line diagnostic
//! summaries. Both the `analyze --events` lens and the `events` command render events at the terminal,
//! so the type-name derivation and the diagnostic-payload summaries live here once rather than being
//! duplicated across the two views.

use zuihitsu::EventPayload;

/// The payload's serde type tag (`ConversationTurn`, `MemoryContentAppended`, …) — the `type` field
/// [`EventPayload`]'s `#[serde(tag = "type")]` writes. Derived from the serialization rather than a
/// hand-maintained match, so a new variant is named correctly without a second edit. Falls back to
/// `Unknown` if the payload does not serialize to a tagged object (it always does).
pub(crate) fn payload_type(payload: &EventPayload) -> String {
    serde_json::to_value(payload)
        .ok()
        .and_then(|value| {
            value
                .get("type")
                .and_then(|tag| tag.as_str())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "Unknown".to_owned())
}

/// A compact one-line summary of a diagnostic event whose fields carry signal for root-causing a run —
/// a wake-up that never fired, an `occurred_at` that landed null or malformed, a description
/// regenerated. Only the variants that carry such signal render; the rest return `None` so a caller can
/// fall back (`analyze --events` filters them out; the `events` command renders its own fallback). The
/// `occurred_at` is rendered as its stored JSON (`{"recurring": ...}`, `{"day": ...}`, or `null`) —
/// exactly the shape that arms or fails to arm a wake-up.
pub(crate) fn diagnostic_summary(payload: &EventPayload) -> Option<String> {
    match payload {
        EventPayload::MemoryContentAppended {
            id,
            occurred_at,
            text,
            visibility,
            ..
        } => Some(format!(
            "append {id:?} occurred_at={} visibility={visibility:?} text={:?}",
            occurred_at
                .as_ref()
                .map(|t| serde_json::to_string(t).unwrap_or_default())
                .unwrap_or_else(|| "null".to_owned()),
            text.trim(),
        )),
        EventPayload::EntryTemporalResolved {
            entry_id,
            occurred_at,
            ..
        } => Some(format!(
            "resolved {entry_id:?} occurred_at={}",
            serde_json::to_string(occurred_at).unwrap_or_default(),
        )),
        EventPayload::ScheduledJobFired {
            memory, fired_at, ..
        } => Some(format!("fired {memory:?} @ {fired_at:?}")),
        EventPayload::ScheduledItemSurfaced {
            memory, session, ..
        } => Some(format!("surfaced {memory:?} in {session:?}")),
        EventPayload::MemorySuperseded {
            entry,
            superseded_by,
            ..
        } => Some(format!("superseded {entry:?} by {superseded_by:?}")),
        EventPayload::MemoryDescriptionRegenerated { id, new_text, .. } => {
            Some(format!("described {id:?}: {:?}", new_text.trim()))
        }
        _ => None,
    }
}
