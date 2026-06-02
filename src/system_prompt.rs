//! System-prompt assembly (spec §System prompt).
//!
//! The frozen system prompt is assembled from the **scaffold** template (the durable, operational
//! framing — how the agent acts, never who it is), the agent's **identity** drawn from `self`, the
//! build-derived **API description** (rendered from the running binary, so the prompt and the
//! implementation cannot drift — see [`crate::lua::render_api_reference`]), and the declared
//! **current time**. The remaining spec source — the per-session **contextual brief** — arrives
//! with the conversation/brief machinery; this composer leaves room for it rather than restating it.
//!
//! Identity is drawn from `self`'s content **entries**, verbatim — not its description. Entries are
//! immutable and append-only, so the authored persona never drifts, while the self still evolves as
//! the agent appends further self-observations; the regenerable description is a lossy summary and
//! is deliberately not the source of the agent's voice.
//!
//! Assembly is a pure function of already-fetched inputs, so the caller owns the store/graph/clock
//! reads (and their error handling) and this stays trivially testable.

use crate::{graph::EntryView, ids::Timestamp};

/// Compose the system prompt from the `scaffold` body, the agent's `identity` (the `self` memory's
/// content entries, verbatim), the `api_reference` block (the build's callable Lua API, rendered by
/// [`crate::lua::render_api_reference`]), the session's frozen contextual `brief` (composed by
/// [`crate::brief::compose`] and captured on `SessionStarted`), and the session's start time `now`.
pub fn assemble(
    scaffold: &str,
    identity: &[EntryView],
    api_reference: &str,
    brief: &str,
    now: Timestamp,
) -> String {
    let mut prompt =
        String::with_capacity(scaffold.len() + api_reference.len() + brief.len() + 256);
    prompt.push_str(scaffold);

    if !identity.is_empty() {
        prompt.push_str("\n\n# Who you are\n\n");
        for (index, entry) in identity.iter().enumerate() {
            if index > 0 {
                prompt.push_str("\n\n");
            }
            prompt.push_str(&entry.text);
        }
    }

    if !api_reference.is_empty() {
        prompt.push_str("\n\n# What you can do\n\n");
        prompt.push_str(api_reference);
    }

    if !brief.is_empty() {
        prompt.push_str("\n\n# What you know right now\n\n");
        prompt.push_str(brief);
    }

    prompt.push_str("\n\n# Current time\n\nThe session begins on ");
    prompt.push_str(&format_time(now));
    prompt.push('.');
    prompt
}

/// Render a timestamp as a human-readable UTC datetime (e.g. `Thursday, 01 January 1970, 00:00
/// UTC`), falling back to raw epoch milliseconds for a time outside the supported range.
fn format_time(now: Timestamp) -> String {
    match jiff::Timestamp::from_millisecond(now.as_millis()) {
        Ok(timestamp) => timestamp
            .to_zoned(jiff::tz::TimeZone::UTC)
            .strftime("%A, %d %B %Y, %H:%M UTC")
            .to_string(),
        Err(_) => format!("{} milliseconds since the Unix epoch", now.as_millis()),
    }
}
