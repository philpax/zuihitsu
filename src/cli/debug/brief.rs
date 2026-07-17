//! The `brief` command: re-compose a session's contextual brief from the inputs its `SessionStarted`
//! recorded, so a change to brief composition can be seen against a real instance's data without
//! re-running the agent (whose frozen brief is baked into the log at session start). The command reads
//! the log read-only — safe while the agent holds the write lock — rebuilds the graph and settings as
//! of the session's start, and prints both the recorded brief and the one the current code produces, so
//! the two can be compared side by side.

use zuihitsu::{
    BriefRequest, Event, EventPayload, Graph, MemoryId, MemoryStore, Seq, Settings, SqliteStore,
    Store, compose, config::EnvConfig,
};

use crate::cli::error::CliError;

/// Which session's brief to reproduce.
pub(crate) enum BriefSelector {
    /// The session active at this event seq — the latest `SessionStarted` at or before it, so a seq a
    /// listing surfaced (`events --seq 99`) resolves to the session that governs it.
    Seq(u64),
    /// A session by its id, or a unique prefix of it.
    Session(String),
}

/// Re-compose the selected session's contextual brief with the current code and print it beside the
/// brief recorded at session start. Opens the log read-only, resolves the target `SessionStarted`,
/// replays every prior event into a scratch in-memory store so the settings and graph match that
/// moment, and composes from the recorded present set, working set, room, and start time.
pub(crate) fn brief(config: &EnvConfig, selector: BriefSelector) -> Result<(), CliError> {
    let path = config.storage.event_log();
    let store = SqliteStore::open_read_only(&path).map_err(|source| {
        CliError::Brief(format!(
            "could not open the event log at {}: {source}",
            path.display()
        ))
    })?;
    let events = store
        .read_from(Seq(0))
        .map_err(|source| CliError::Brief(format!("could not read the event log: {source}")))?;

    let target = resolve_session(&events, &selector)?;
    let start_seq = target.seq;
    let EventPayload::SessionStarted {
        conversation,
        id,
        participants,
        started_at,
        brief: recorded,
        working_set,
        initiators,
        ..
    } = &target.payload
    else {
        unreachable!("resolve_session only returns a SessionStarted event");
    };

    // Rebuild the state the brief saw: every event strictly before its `SessionStarted`, replayed into
    // a fresh in-memory store, so the settings and graph reflect that moment rather than the current
    // head (the brief is composed just before the `SessionStarted` is appended).
    let mut prior = MemoryStore::new();
    for event in events.iter().filter(|event| event.seq < start_seq) {
        prior
            .append(
                event.recorded_at,
                event.source.clone(),
                vec![event.payload.clone()],
            )
            .map_err(|source| {
                CliError::Brief(format!("could not replay the prior log: {source}"))
            })?;
    }
    let settings = Settings::from_store(&prior)
        .map_err(|source| CliError::Brief(format!("could not read the settings: {source}")))?;
    let mut graph = Graph::open_in_memory()
        .map_err(|source| CliError::Brief(format!("could not open a scratch graph: {source}")))?;
    graph
        .materialize_from(&prior)
        .map_err(|source| CliError::Brief(format!("could not materialize the graph: {source}")))?;
    let context = graph
        .context_for_conversation(*conversation)
        .map_err(|source| CliError::Brief(format!("could not resolve the room: {source}")))?;

    let fresh = compose(
        &graph,
        &settings.brief,
        &BriefRequest {
            present_set: participants,
            speakers: initiators,
            current_context: context,
            working_set,
            now: *started_at,
        },
    )
    .map_err(|source| CliError::Brief(format!("could not compose the brief: {source}")))?;

    // A header naming what was reproduced, then the two briefs. The recorded one is the exact text
    // frozen into the session's system prompt; the recomposed one is what the current code produces
    // from the same inputs.
    let room = context
        .and_then(|id| graph.memory_by_id(id).ok().flatten())
        .map_or_else(
            || "(no room)".to_owned(),
            |memory| memory.name.as_str().to_owned(),
        );
    println!(
        "session {} · started at seq {} · room {room}",
        id.0, start_seq.0
    );
    println!("present: {}", names(&graph, participants));
    println!("working set: {}", names(&graph, working_set));
    println!("\n── recorded brief (frozen at session start) ──\n{recorded}");
    println!("── recomposed with the current code ──\n{fresh}");
    Ok(())
}

/// Resolve the selector to its `SessionStarted` event: an exact-or-prefix id match, or the latest
/// session started at or before a seq.
fn resolve_session<'a>(
    events: &'a [Event],
    selector: &BriefSelector,
) -> Result<&'a Event, CliError> {
    let starts = events
        .iter()
        .filter(|event| matches!(event.payload, EventPayload::SessionStarted { .. }));
    match selector {
        BriefSelector::Session(query) => starts
            .filter(|event| session_id(event).is_some_and(|id| id.starts_with(query)))
            .min_by_key(|event| event.seq.0)
            .ok_or_else(|| CliError::Brief(format!("no session with id {query}"))),
        BriefSelector::Seq(seq) => starts
            .filter(|event| event.seq.0 <= *seq)
            .max_by_key(|event| event.seq.0)
            .ok_or_else(|| CliError::Brief(format!("no session started at or before seq {seq}"))),
    }
}

/// The `SessionStarted` id as a string, for the selector's id match.
fn session_id(event: &Event) -> Option<String> {
    match &event.payload {
        EventPayload::SessionStarted { id, .. } => Some(id.0.to_string()),
        _ => None,
    }
}

/// Render a set of memory ids as their handles, resolving each against the graph and falling back to
/// the raw id when a memory is unknown (a soft-deleted or never-created id).
fn names(graph: &Graph, ids: &[MemoryId]) -> String {
    if ids.is_empty() {
        return "(none)".to_owned();
    }
    ids.iter()
        .map(|id| {
            graph.memory_by_id(*id).ok().flatten().map_or_else(
                || id.0.to_string(),
                |memory| memory.name.as_str().to_owned(),
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::{BriefSelector, resolve_session, session_id};
    use zuihitsu::{ConversationId, Event, EventPayload, EventSource, Seq, SessionId, Timestamp};

    /// A minimal `SessionStarted` event at `seq`, enough to exercise the selector.
    fn session_started(seq: u64) -> Event {
        Event {
            seq: Seq(seq),
            recorded_at: Timestamp::from_millis(seq as i64),
            source: EventSource::Orchestration,
            payload: EventPayload::SessionStarted {
                conversation: ConversationId::generate(),
                id: SessionId::generate(),
                participants: Vec::new(),
                started_at: Timestamp::from_millis(seq as i64),
                seeded_from_turn: None,
                brief: String::new(),
                working_set: Vec::new(),
                initiators: Vec::new(),
            },
        }
    }

    #[test]
    fn seq_resolves_to_the_session_active_at_that_point() {
        let events = vec![
            session_started(10),
            session_started(50),
            session_started(90),
        ];
        // A seq between two sessions resolves to the latest one at or before it.
        assert_eq!(
            resolve_session(&events, &BriefSelector::Seq(70))
                .unwrap()
                .seq,
            Seq(50)
        );
        // A seq exactly on a boundary picks that session.
        assert_eq!(
            resolve_session(&events, &BriefSelector::Seq(90))
                .unwrap()
                .seq,
            Seq(90)
        );
        // A seq before the first session has no governing session.
        assert!(resolve_session(&events, &BriefSelector::Seq(5)).is_err());
    }

    #[test]
    fn a_session_id_prefix_resolves_uniquely() {
        let event = session_started(10);
        let full = session_id(&event).unwrap();
        let prefix = full[..8].to_owned();
        let events = std::slice::from_ref(&event);
        assert_eq!(
            resolve_session(events, &BriefSelector::Session(prefix))
                .unwrap()
                .seq,
            Seq(10)
        );
        // An id that matches nothing is an error rather than a wrong session.
        assert!(resolve_session(events, &BriefSelector::Session("zzzzzzzz".to_owned())).is_err());
    }
}
