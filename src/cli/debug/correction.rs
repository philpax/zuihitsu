//! The `retract` and `clear-occurrence` correction commands: the two deliberate write exceptions in the
//! otherwise read-only `debug` namespace. Each appends one forward, operator-sourced event to the log —
//! `retract` withdraws an entry outright to a tombstone, and `clear-occurrence` withdraws an entry's
//! resolved occurrence so it returns to untimed (disarming any wake-up the occurrence armed). Both open
//! the event log read-write, so the agent must be stopped first: the open takes the single-writer log
//! lock and fails while a running agent holds it. Unlike `revert`, they rewrite no history — the fix is
//! recorded as a new event, auditable and itself revertible.
//!
//! The two share [`resolve_entry`]: an operator-typed entry id or unique prefix is resolved against a
//! graph freshly materialised from the log, erroring when the prefix is ambiguous (listing the
//! candidates) or matches nothing.

use zuihitsu::{
    Clock, EntryView, EventPayload, EventSource, Graph, MemoryView, SqliteStore, Store,
    SystemClock, config::EnvConfig, format_occurrence,
};

use crate::cli::error::CliError;

/// Retract a live entry to a tombstone, recording why: append an operator-sourced `EntryRetracted`, so
/// the entry drops from every live surface on the next fold while its content stays in the log for
/// audit. Resolves the entry by id or unique prefix, refuses an empty reason, and refuses an entry that
/// is already superseded or retracted (there is nothing live to withdraw).
pub(crate) fn retract(config: &EnvConfig, target: &str, reason: &str) -> Result<(), CliError> {
    let reason = reason.trim();
    if reason.is_empty() {
        return Err(CliError::Retract(
            "a retraction reason is required — an unaudited withdrawal is unauditable".to_owned(),
        ));
    }

    let mut store = open_store(config, CliError::Retract)?;
    let graph = materialize(&store, CliError::Retract)?;
    let resolved = resolve_entry(&graph, target).map_err(CliError::Retract)?;

    if resolved.entry.superseded_by.is_some() {
        return Err(CliError::Retract(format!(
            "entry {} on {} is already superseded or retracted; nothing live to withdraw",
            resolved.entry.entry_id.0,
            resolved.memory.name.as_str(),
        )));
    }

    store
        .append(
            SystemClock.now(),
            EventSource::Operator,
            vec![EventPayload::entry_retracted(
                resolved.memory.id,
                resolved.entry.entry_id,
                reason.to_owned(),
                None,
            )],
        )
        .map_err(|source| {
            CliError::Retract(format!("could not append the retraction: {source}"))
        })?;

    tracing::info!(
        "retracted entry {} on {} ({}): {:?} — reason: {reason}. It drops from every live surface on \
         the next fold; its content stays in the log.",
        resolved.entry.entry_id.0,
        resolved.memory.name.as_str(),
        resolved.memory.id.0,
        snippet(&resolved.entry.text),
    );
    Ok(())
}

/// Clear an entry's resolved occurrence: append an operator-sourced `EntryTemporalResolved` carrying
/// `None`, so the entry returns to untimed and any wake-up its occurrence armed is disarmed. Resolves
/// the entry by id or unique prefix, and refuses an entry that carries no occurrence to clear.
pub(crate) fn clear_occurrence(config: &EnvConfig, target: &str) -> Result<(), CliError> {
    let mut store = open_store(config, CliError::ClearOccurrence)?;
    let graph = materialize(&store, CliError::ClearOccurrence)?;
    let resolved = resolve_entry(&graph, target).map_err(CliError::ClearOccurrence)?;

    let Some(occurrence) = resolved.entry.occurred_at.as_ref() else {
        return Err(CliError::ClearOccurrence(format!(
            "entry {} on {} carries no occurrence to clear",
            resolved.entry.entry_id.0,
            resolved.memory.name.as_str(),
        )));
    };
    let cleared = format_occurrence(occurrence);

    store
        .append(
            SystemClock.now(),
            EventSource::Operator,
            vec![EventPayload::entry_temporal_resolved(
                resolved.memory.id,
                resolved.entry.entry_id,
                None,
                None,
            )],
        )
        .map_err(|source| {
            CliError::ClearOccurrence(format!(
                "could not append the occurrence withdrawal: {source}"
            ))
        })?;

    tracing::info!(
        "cleared the occurrence ({cleared}) from entry {} on {} ({}): {:?}. The entry returns to \
         untimed and any wake-up it armed is disarmed on the next fold.",
        resolved.entry.entry_id.0,
        resolved.memory.name.as_str(),
        resolved.memory.id.0,
        snippet(&resolved.entry.text),
    );
    Ok(())
}

/// An entry resolved for a correction command: its owning memory and its projected view.
#[derive(Debug)]
struct ResolvedEntry {
    memory: MemoryView,
    entry: EntryView,
}

/// Resolve `target` — a full entry id or a unique prefix of one — to exactly one live-memory entry. An
/// ambiguous prefix is an error that lists the candidates; a prefix matching nothing, or one whose only
/// match sits on a soft-deleted memory, is an error too. The error is a bare message the caller wraps in
/// its own command context.
fn resolve_entry(graph: &Graph, target: &str) -> Result<ResolvedEntry, String> {
    let candidates = graph
        .entry_ids_with_prefix(target)
        .map_err(|source| format!("could not resolve the entry: {source}"))?;
    match candidates.as_slice() {
        [] => Err(format!("no entry found with id or prefix {target:?}")),
        [id] => {
            let (memory, entry) = graph
                .entry_by_id(*id)
                .map_err(|source| format!("could not read the entry: {source}"))?
                .ok_or_else(|| format!("entry {} resolves to a soft-deleted memory", id.0))?;
            Ok(ResolvedEntry { memory, entry })
        }
        many => Err(format!(
            "ambiguous prefix {target:?} matches {} entries: {}",
            many.len(),
            many.iter()
                .map(|id| id.0.to_string())
                .collect::<Vec<_>>()
                .join(", "),
        )),
    }
}

/// Open the event log read-write, failing (with a running-agent hint) when the single-writer lock is
/// already held. `wrap` builds the command's own error variant so the context prefix stays correct.
/// Shared with the sibling identity commands ([`crate::cli::debug::identity`]), the other write
/// exceptions that take the single-writer lock.
pub(super) fn open_store(
    config: &EnvConfig,
    wrap: fn(String) -> CliError,
) -> Result<SqliteStore, CliError> {
    let log_path = config.storage.event_log();
    SqliteStore::open(&log_path).map_err(|source| {
        wrap(format!(
            "could not open the event log at {} for writing (is the agent running?): {source}",
            log_path.display()
        ))
    })
}

/// Materialise a scratch graph from the log, so the target resolves against current state. Shared with
/// the sibling identity commands ([`crate::cli::debug::identity`]).
pub(super) fn materialize(
    store: &SqliteStore,
    wrap: fn(String) -> CliError,
) -> Result<Graph, CliError> {
    let mut graph = Graph::open_in_memory()
        .map_err(|source| wrap(format!("could not open a scratch graph: {source}")))?;
    graph
        .materialize_from(store)
        .map_err(|source| wrap(format!("could not materialize the graph: {source}")))?;
    Ok(graph)
}

/// A short single-line preview of an entry's text for the operator's confirmation log — clipped so a
/// long entry does not flood the terminal.
fn snippet(text: &str) -> String {
    const MAX: usize = 80;
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    match flat.char_indices().nth(MAX) {
        Some((byte, _)) => format!("{}…", &flat[..byte]),
        None => flat,
    }
}

#[cfg(test)]
mod tests {
    //! In-memory, like the other `debug` command tests: they exercise the resolution and refusal logic
    //! over a folded log rather than opening a real store. The disk plumbing (opening the log under the
    //! lock, appending the event) is the same thin shell the sibling commands leave untested; the
    //! `EntryRetracted` and withdrawing `EntryTemporalResolved` folds are covered by the graph's tests.
    use super::{resolve_entry, snippet};
    use zuihitsu::{
        Clock, EntryId, EventPayload, EventSource, Graph, MemoryId, MemoryStore, Namespace, Store,
        SystemClock, Teller, Timestamp, Visibility,
    };

    /// A graph materialized from an in-memory log of `payloads`.
    fn graph_of(payloads: Vec<EventPayload>) -> Graph {
        let mut store = MemoryStore::new();
        for payload in payloads {
            store
                .append(SystemClock.now(), EventSource::Operator, vec![payload])
                .unwrap();
        }
        let mut graph = Graph::open_in_memory().unwrap();
        graph.materialize_from(&store).unwrap();
        graph
    }

    fn seeded(entry: EntryId, name: &str, text: &str) -> (MemoryId, Vec<EventPayload>) {
        let memory = MemoryId::generate();
        (
            memory,
            vec![
                EventPayload::memory_created(memory, Namespace::Event.with_name(name)),
                EventPayload::MemoryContentAppended {
                    id: memory,
                    entry_id: entry,
                    asserted_at: Timestamp::from_millis(1),
                    occurred_at: None,
                    text: text.to_owned(),
                    told_by: Teller::Agent,
                    told_in: None,
                    visibility: Visibility::Public,
                },
            ],
        )
    }

    #[test]
    fn an_entry_resolves_by_full_id_and_by_unique_prefix() {
        let entry = EntryId::generate();
        let (_memory, payloads) = seeded(entry, "standup", "a live fact");
        let graph = graph_of(payloads);
        let full = entry.0.to_string();
        assert_eq!(resolve_entry(&graph, &full).unwrap().entry.entry_id, entry);
        // A prefix of the id resolves the same entry, in either casing.
        let prefix = &full[..10];
        assert_eq!(resolve_entry(&graph, prefix).unwrap().entry.entry_id, entry);
        assert_eq!(
            resolve_entry(&graph, &prefix.to_lowercase())
                .unwrap()
                .entry
                .entry_id,
            entry
        );
    }

    #[test]
    fn an_unknown_prefix_is_an_error() {
        let entry = EntryId::generate();
        let (_memory, payloads) = seeded(entry, "standup", "a live fact");
        let graph = graph_of(payloads);
        let error = resolve_entry(&graph, "00000000000000000000000000").unwrap_err();
        assert!(error.contains("no entry found"), "got: {error}");
    }

    #[test]
    fn an_ambiguous_prefix_lists_the_candidates() {
        // Two entries sharing a common id prefix — resolving that prefix is ambiguous. ULIDs lead with a
        // timestamp, so entries minted close together already share a long prefix; the empty string is
        // the guaranteed-common prefix that matches both.
        let a = EntryId::generate();
        let b = EntryId::generate();
        let (_ma, mut payloads) = seeded(a, "standup", "first fact");
        let (_mb, more) = seeded(b, "review", "second fact");
        payloads.extend(more);
        let graph = graph_of(payloads);
        let error = resolve_entry(&graph, "").unwrap_err();
        assert!(error.contains("ambiguous prefix"), "got: {error}");
        assert!(error.contains(&a.0.to_string()), "got: {error}");
        assert!(error.contains(&b.0.to_string()), "got: {error}");
    }

    #[test]
    fn snippet_clips_long_text_and_flattens_whitespace() {
        assert_eq!(snippet("  a   short  fact "), "a short fact");
        let long = "word ".repeat(40);
        let clipped = snippet(&long);
        assert!(clipped.ends_with('…') && clipped.len() < long.len());
    }
}
