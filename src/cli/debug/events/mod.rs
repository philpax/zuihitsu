//! The `events` command's log inspection and rendering: the filtered listing, the per-event
//! summary and session timeline, and the payload descriptions the CLI prints (diagnostic
//! output; the operator-facing view of the raw event log).

use std::{collections::BTreeMap, io::Write};

use anstyle::{AnsiColor, Style};
use zuihitsu::{
    Event, MemoryId, Seq, SqliteStore, Store,
    config::EnvConfig,
    event::{EventPayload, EventSource, Teller, TerminalCause, TurnRole, Visibility},
    time::format_iso8601,
};

use crate::cli::error::CliError;

/// The filters and output mode for the `events` command, bundled so the inspection call stays one
/// argument rather than a fistful of options.
pub(crate) struct EventQuery<'a> {
    /// Show this one event's full payload, pretty-printed (ignores the rest).
    pub(crate) seq: Option<u64>,
    pub(crate) from: Option<u64>,
    pub(crate) to: Option<u64>,
    pub(crate) type_: &'a Option<String>,
    pub(crate) target: &'a Option<String>,
    pub(crate) json: bool,
    pub(crate) summary: bool,
}

/// Inspect the event log directly, opening it read-only so it is safe to read while the agent holds
/// the write lock. With `seq`, pretty-prints that one event's full payload; otherwise lists each event
/// (seq, type, and its target) or, with `summary`, counts events by type and lays out the session
/// timeline. `from`/`to` bound the seq range, `type_` filters by event type, `target` by the
/// conversation or memory the event is about, and `json` prints full payloads in the listing.
pub(crate) fn events(config: &EnvConfig, query: EventQuery) -> Result<(), CliError> {
    let EventQuery {
        seq,
        from,
        to,
        type_,
        target,
        json,
        summary,
    } = query;
    let path = config.storage.event_log();
    let store = SqliteStore::open_read_only(&path).map_err(|source| {
        CliError::Events(format!(
            "could not open the event log at {}: {source}",
            path.display()
        ))
    })?;
    let events = store
        .read_from(Seq(0))
        .map_err(|source| CliError::Events(format!("could not read the event log: {source}")))?;

    // A single event by seq: its whole payload, pretty-printed — the zoom-in the listing points at.
    if let Some(seq) = seq {
        let event = events
            .iter()
            .find(|event| event.seq.0 == seq)
            .ok_or_else(|| CliError::Events(format!("no event at seq {seq}")))?;
        let payload = serde_json::to_string_pretty(&event.payload).map_err(|source| {
            CliError::Events(format!("could not render the payload: {source}"))
        })?;
        println!(
            "seq {} · {} · {}\n{payload}",
            event.seq.0,
            format_iso8601(event.recorded_at),
            event.payload.kind()
        );
        return Ok(());
    }

    // Resolve memory ids to names from the create/rename events across the WHOLE log, before the
    // filters narrow the view — so a link or append in a slice still reads in names even when the
    // memory was created outside the slice. The log is self-describing, so no graph read is needed.
    let names = name_map(&events);

    let mut events = events;
    if let Some(from) = from {
        events.retain(|event| event.seq.0 >= from);
    }
    if let Some(to) = to {
        events.retain(|event| event.seq.0 <= to);
    }
    if let Some(type_) = type_ {
        events.retain(|event| event.payload.kind().eq_ignore_ascii_case(type_));
    }
    if let Some(target) = target {
        events.retain(|event| {
            event
                .payload
                .target_id()
                .is_some_and(|id| id.starts_with(target.as_str()))
        });
    }

    if summary {
        print_event_summary(&events);
    } else if json {
        // Clean NDJSON: one whole event per line (seq, recorded_at, payload), no columns, so the
        // output pipes straight into `jq` or any line-oriented parser.
        for event in &events {
            let line = serde_json::to_string(event).map_err(|source| {
                CliError::Events(format!(
                    "could not serialize event {}: {source}",
                    event.seq.0
                ))
            })?;
            println!("{line}");
        }
    } else {
        let mut out = anstream::stdout();
        for event in &events {
            let _ = write_event(&mut out, event, &names, false);
        }
    }
    Ok(())
}

/// Write one event as the two-line listing — a dim seq, the kind in its category color, then the
/// payload gloss — to `out`. When `faded`, the whole event is dimmed instead of colored. The shared
/// renderer for the `events` listing (never faded) and the `revert` preview (which fades the events it
/// would remove). `anstream::stdout` adapts the styling to the destination — colored on a terminal
/// (including Windows), stripped when piped, off under `NO_COLOR` — so there is no manual terminal
/// gate; an `anstyle::Style` renders its prefix as `{style}` and its reset as `{style:#}`.
pub(crate) fn write_event(
    out: &mut impl Write,
    event: &Event,
    names: &BTreeMap<String, String>,
    faded: bool,
) -> std::io::Result<()> {
    let dim = Style::new().dimmed();
    let kind_style = if faded {
        dim
    } else {
        Style::new().fg_color(Some(category_color(&event.payload).into()))
    };
    let kind = event.payload.kind();
    // The envelope authority, dimly, for every non-`Agent` event: the agent authors the bulk of a
    // log, so annotating only the exceptions keeps the listing calm while a genesis, operator, or
    // system write stands out.
    let source = match &event.source {
        EventSource::Agent => String::new(),
        other => format!("  [{}]", other.as_str().to_lowercase()),
    };
    // The wall-clock stamp rides dim beside the seq, so a glance sees both the log position and *when*
    // the write landed without decoding the payload.
    let stamp = format_iso8601(event.recorded_at);
    writeln!(
        out,
        "{dim}{:>6} {stamp}{dim:#} {kind_style}{kind}{kind_style:#}{dim}{source}{dim:#}",
        event.seq.0
    )?;
    let detail = describe_event(&event.payload, names);
    if !detail.is_empty() {
        if faded {
            writeln!(out, "        {dim}{detail}{dim:#}")?;
        } else {
            writeln!(out, "        {detail}")?;
        }
    }
    Ok(())
}

/// The latest name of every memory the log creates or renames, keyed by its id string — the lookup
/// `describe_event` uses to render an event in names instead of ULIDs.
pub(crate) fn name_map(events: &[Event]) -> BTreeMap<String, String> {
    let mut names = BTreeMap::new();
    for event in events {
        match &event.payload {
            EventPayload::MemoryCreated { id, name } => {
                names.insert(id.0.to_string(), name.as_str().to_owned());
            }
            EventPayload::MemoryRenamed { id, new_name, .. } => {
                names.insert(id.0.to_string(), new_name.as_str().to_owned());
            }
            _ => {}
        }
    }
    names
}

/// A human-readable one-line gloss of an event — what happened, with memory ids resolved to names —
/// so the timeline reads without decoding JSON, the way the console viewer renders it. Falls back to
/// the bare event kind for the structural events with no salient content to show.
fn describe_event(payload: &EventPayload, names: &BTreeMap<String, String>) -> String {
    let name = |id: &MemoryId| {
        names
            .get(&id.0.to_string())
            .cloned()
            .unwrap_or_else(|| short_id(&id.0.to_string()))
    };
    match payload {
        EventPayload::ConversationTurn {
            role,
            text,
            participant,
            ..
        } => {
            let who = match (role, participant) {
                (TurnRole::Agent, _) => "agent".to_owned(),
                (TurnRole::System, _) => "system".to_owned(),
                (TurnRole::Participant, Some(id)) => name(id),
                (TurnRole::Participant, None) => "participant".to_owned(),
            };
            format!("«{who}» {}", oneline(text, 90))
        }
        EventPayload::MemoryContentAppended {
            id,
            text,
            visibility,
            told_by,
            ..
        } => format!(
            "{} ← \"{}\"  [{}, {}]",
            name(id),
            oneline(text, 60),
            visibility_label(visibility),
            teller_label(told_by, &name),
        ),
        EventPayload::LuaExecuted {
            result,
            terminal_cause,
            ..
        } => match (terminal_cause, result) {
            (Some(TerminalCause::Error(message)), _) => format!("error: {}", oneline(message, 80)),
            (Some(TerminalCause::Aborted(reason)), _) => {
                format!("aborted: {}", oneline(reason, 80))
            }
            (Some(TerminalCause::Skipped(reason)), _) => {
                format!("skipped: {}", oneline(reason.as_deref().unwrap_or(""), 80))
            }
            (None, Some(result)) => oneline(result, 100),
            (None, None) => String::new(),
        },
        EventPayload::MemoryCreated { name: created, .. } => {
            format!("created {}", created.as_str())
        }
        EventPayload::MemoryRenamed {
            old_name, new_name, ..
        } => format!("renamed {} → {}", old_name.as_str(), new_name.as_str()),
        EventPayload::MemoryDeleted { id } => format!("deleted {}", name(id)),
        EventPayload::MemorySuperseded { id, .. } => format!("{}: superseded an entry", name(id)),
        EventPayload::EntriesConsolidated { id, sources, .. } => {
            format!("{}: consolidated {} entries", name(id), sources.len())
        }
        EventPayload::EntryRetracted { memory, reason, .. } => {
            format!("{}: retracted an entry ({reason})", name(memory))
        }
        EventPayload::MemoryDescriptionRegenerated { id, .. } => {
            format!("{}: re-described", name(id))
        }
        EventPayload::BeliefArbitrated {
            memory,
            competing_entries,
            ..
        } => format!(
            "{}: arbitrated ({} competing)",
            name(memory),
            competing_entries.len()
        ),
        EventPayload::MemoryVolatilitySet { id, volatility } => {
            format!("{}: volatility {}", name(id), volatility)
        }
        EventPayload::LinkCreated {
            from, to, relation, ..
        } => format!("{} —{}→ {}", name(from), relation.as_str(), name(to)),
        EventPayload::LinkRemoved {
            from, to, relation, ..
        } => format!("{} —{}✗ {}", name(from), relation.as_str(), name(to)),
        EventPayload::TagAppliedToMemory { memory, tag } => {
            format!("{} +#{}", name(memory), tag.as_str())
        }
        EventPayload::TagRemovedFromMemory { memory, tag } => {
            format!("{} −#{}", name(memory), tag.as_str())
        }
        EventPayload::TagCreated { name: tag, .. } => format!("created #{}", tag.as_str()),
        EventPayload::LinkTypeRegistered {
            name: rel, inverse, ..
        } => {
            format!("registered relation {}/{}", rel.as_str(), inverse.as_str())
        }
        EventPayload::ScheduledJobFired { memory, .. } => {
            format!("wake-up fired ({})", name(memory))
        }
        EventPayload::ScheduledItemSurfaced { memory, .. } => {
            format!("wake-up surfaced ({})", name(memory))
        }
        EventPayload::EntryTemporalResolved { id, .. } => format!("{}: resolved a date", name(id)),
        EventPayload::EntryTemporalResolveFailed { id, .. } => {
            format!("{}: failed to resolve a date", name(id))
        }
        EventPayload::MergeProposed {
            from, to, source, ..
        } => {
            format!(
                "merge proposed ({}): {} → {}",
                source.as_str().to_lowercase(),
                name(from),
                name(to)
            )
        }
        EventPayload::ModelCalled { phase, .. } => format!("{phase:?}"),
        EventPayload::ModelCallAborted { attempt, cause, .. } => {
            format!("attempt {attempt} discarded: {cause}")
        }
        EventPayload::AmbientRecallSurfaced { hits, .. } => {
            format!("ambient recall: {} memories", hits.len())
        }
        EventPayload::TurnSuperseded { turn_id, .. } => {
            format!("turn {} superseded before its reply", turn_id.0)
        }
        EventPayload::EmbeddingModelChanged { from, to } => {
            format!("embedding model {from} → {to}")
        }
        EventPayload::GenesisCompleted {
            manifest_hash,
            template_versions,
        } => format!(
            "manifest {}…, {} templates",
            &manifest_hash[..manifest_hash.len().min(10)],
            template_versions.len()
        ),
        EventPayload::PromptTemplateRegistered {
            name: template,
            version,
            ..
        } => format!("{template:?} v{version}"),
        EventPayload::TagDescriptionChanged {
            name: tag,
            new_description,
        } => format!("#{}: \"{}\"", tag.as_str(), oneline(new_description, 60)),
        EventPayload::ConfigSet { .. } => "settings updated".to_owned(),
        // The remaining events — session and conversation boundaries, soft deletes — are markers whose
        // kind on the first line is the whole story; they get no second line.
        _ => String::new(),
    }
}

/// Collapse whitespace runs (so a multi-line script or reply stays one line) and clip to `max` chars.
fn oneline(text: &str, max: usize) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() > max {
        let kept: String = collapsed.chars().take(max).collect();
        format!("{kept}…")
    } else {
        collapsed
    }
}

/// The last segment of a ULID, for a memory the log never names (it shows enough to disambiguate).
fn short_id(id: &str) -> String {
    format!("…{}", &id[id.len().saturating_sub(6)..])
}

fn visibility_label(visibility: &Visibility) -> &'static str {
    match visibility {
        Visibility::Public => "public",
        Visibility::Attributed => "attributed",
        Visibility::PrivateToTeller => "private",
        Visibility::Exclude(_) => "excluded",
    }
}

fn teller_label(teller: &Teller, name: &impl Fn(&MemoryId) -> String) -> String {
    match teller {
        Teller::Participant(id) => name(id),
        Teller::Agent => "agent".to_owned(),
        Teller::Bootstrap => "seed".to_owned(),
    }
}

/// The color an event's kind is rendered in, grouped so a glance separates conversation from writes
/// from relations from belief from telemetry. Matched on the variant (not its name), so adding an
/// event kind is a compile error here until it is given a category.
fn category_color(payload: &EventPayload) -> AnsiColor {
    match payload {
        // Conversation and session flow.
        EventPayload::ConversationTurn { .. }
        | EventPayload::ConversationStarted { .. }
        | EventPayload::ConversationEnded { .. }
        | EventPayload::SessionStarted { .. }
        | EventPayload::SessionEnded { .. }
        | EventPayload::ParticipantJoined { .. } => AnsiColor::Cyan,
        // A recorded fact — the substance of the log.
        EventPayload::MemoryContentAppended { .. } => AnsiColor::Green,
        // Memory structure and the entry lifecycle.
        EventPayload::MemoryCreated { .. }
        | EventPayload::MemoryRenamed { .. }
        | EventPayload::MemoryDeleted { .. }
        | EventPayload::MemorySuperseded { .. }
        | EventPayload::EntriesConsolidated { .. }
        | EventPayload::EntryRetracted { .. }
        | EventPayload::EntryAttested { .. }
        | EventPayload::AttestationRetracted { .. }
        | EventPayload::MemoryDescriptionRegenerated { .. }
        | EventPayload::MemoryVolatilitySet { .. }
        | EventPayload::EntryTemporalResolved { .. }
        | EventPayload::EntryTemporalResolveFailed { .. }
        | EventPayload::EntryDescriptionMirrored { .. } => AnsiColor::BrightGreen,
        // Relations and cross-platform identity.
        EventPayload::LinkCreated { .. }
        | EventPayload::LinkRemoved { .. }
        | EventPayload::LinkTypeRegistered { .. }
        | EventPayload::ClassPrimaryDesignated { .. }
        | EventPayload::ParticipantIdentified { .. } => AnsiColor::Blue,
        // Tags and scheduling.
        EventPayload::TagCreated { .. }
        | EventPayload::TagDescriptionChanged { .. }
        | EventPayload::TagAppliedToMemory { .. }
        | EventPayload::TagRemovedFromMemory { .. }
        | EventPayload::ScheduledJobFired { .. }
        | EventPayload::ScheduledItemSurfaced { .. } => AnsiColor::Yellow,
        // Belief arbitration and merges.
        EventPayload::BeliefArbitrated { .. }
        | EventPayload::MergeProposed { .. }
        | EventPayload::LinksInferred { .. }
        | EventPayload::DescribePassCompleted { .. } => AnsiColor::Magenta,
        // Telemetry and structural or config events — the quiet background.
        EventPayload::ModelCalled { .. }
        | EventPayload::ModelCallAborted { .. }
        | EventPayload::AmbientRecallSurfaced { .. }
        | EventPayload::TurnSuperseded { .. }
        | EventPayload::LuaExecuted { .. }
        | EventPayload::GenesisCompleted { .. }
        | EventPayload::ConfigSet { .. }
        | EventPayload::PromptTemplateRegistered { .. }
        | EventPayload::EmbeddingModelChanged { .. } => AnsiColor::BrightBlack,
    }
}

/// Counts by event type (commonest first), then every session boundary (`SessionStarted` /
/// `SessionEnded`) with its conversation — so dangling or duplicated sessions are visible at a glance.
fn print_event_summary(events: &[Event]) {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for event in events {
        *counts.entry(event.payload.kind()).or_default() += 1;
    }
    let mut by_count: Vec<(&str, usize)> = counts.into_iter().collect();
    by_count.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));

    println!("{} events", events.len());
    for (kind, count) in by_count {
        println!("  {count:>5}  {kind}");
    }

    println!("\nsessions");
    let mut any = false;
    for event in events {
        match &event.payload {
            EventPayload::SessionStarted {
                conversation,
                brief,
                ..
            } => {
                any = true;
                println!(
                    "  seq {:>5}  {}  started  {}  brief {}ch",
                    event.seq.0,
                    format_iso8601(event.recorded_at),
                    conversation.0,
                    brief.len()
                );
            }
            EventPayload::SessionEnded { conversation, .. } => {
                any = true;
                println!(
                    "  seq {:>5}  {}  ended    {}",
                    event.seq.0,
                    format_iso8601(event.recorded_at),
                    conversation.0
                );
            }
            _ => {}
        }
    }
    if !any {
        println!("  (none)");
    }
}

#[cfg(test)]
mod tests;
