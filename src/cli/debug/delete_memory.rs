//! The `delete-memory` command: soft-delete a memory by appending a `MemoryDeleted` tombstone, so it
//! drops from every live surface — the graph, search, and the console — on the next fold. Its contents
//! stay in the log (a soft delete preserves history), so this hides the memory without rewriting the
//! past; unlike `revert`, it appends forward rather than truncating. Opens the log read-write, so the
//! agent must be stopped first, and requires `--yes` to proceed.

use zuihitsu::{
    Clock, EventPayload, EventSource, Graph, MemoryId, MemoryName, MemoryView, SqliteStore, Store,
    SystemClock, config::EnvConfig,
};

use crate::cli::error::CliError;

pub(crate) fn delete_memory(config: &EnvConfig, target: &str, yes: bool) -> Result<(), CliError> {
    let log_path = config.storage.event_log();
    let mut store = SqliteStore::open(&log_path).map_err(|source| {
        CliError::DeleteMemory(format!(
            "could not open the event log at {} for writing (is the agent running?): {source}",
            log_path.display()
        ))
    })?;

    // Materialize a scratch graph to resolve the target and confirm it is still live (a memory already
    // deleted no longer resolves, so a second run is a clear no-op rather than a duplicate tombstone).
    let mut graph = Graph::open_in_memory().map_err(|source| {
        CliError::DeleteMemory(format!("could not open a scratch graph: {source}"))
    })?;
    graph.materialize_from(&store).map_err(|source| {
        CliError::DeleteMemory(format!("could not materialize the graph: {source}"))
    })?;

    let memory = resolve(&graph, target)?;
    let entries = graph.entries_local(memory.id).map_err(|source| {
        CliError::DeleteMemory(format!("could not read the memory's entries: {source}"))
    })?;
    let plural = if entries.len() == 1 { "y" } else { "ies" };

    if !yes {
        tracing::info!(
            "would soft-delete {} ({}) — {} entr{plural}",
            memory.name.as_str(),
            memory.id.0,
            entries.len(),
        );
        tracing::warn!(
            "re-run with --yes to confirm; it drops the memory from the graph, search, and the \
             console, but its contents stay in the log"
        );
        // Deleting a conversation's room drops the whole conversation, so a later message to that
        // room opens a fresh, empty one — its prior history does not carry over.
        if memory.name.as_str().starts_with("context/") {
            tracing::warn!(
                "{} is a conversation's room: deleting it drops the conversation, and a later \
                 message to that room opens a fresh one with no carried-over history",
                memory.name.as_str(),
            );
        }
        return Ok(());
    }

    store
        .append(
            SystemClock.now(),
            EventSource::Operator,
            vec![EventPayload::memory_deleted(memory.id)],
        )
        .map_err(|source| {
            CliError::DeleteMemory(format!("could not append the tombstone: {source}"))
        })?;

    tracing::info!(
        "soft-deleted {} ({}) with {} entr{plural}; it drops from the graph, search, and the console \
         on the next fold, and the agent applies it on its next boot. Its contents remain in the log.",
        memory.name.as_str(),
        memory.id.0,
        entries.len(),
    );
    Ok(())
}

/// Resolve a target to a live memory: its exact name first (e.g. `context/console:lua`), then its
/// full id. A deleted or unknown memory resolves to neither.
fn resolve(graph: &Graph, target: &str) -> Result<MemoryView, CliError> {
    if let Some(memory) = graph
        .memory_by_name(MemoryName::new(target))
        .map_err(|source| CliError::DeleteMemory(format!("could not look up the name: {source}")))?
    {
        return Ok(memory);
    }
    if let Ok(ulid) = target.parse::<ulid::Ulid>()
        && let Some(memory) = graph.memory_by_id(MemoryId(ulid)).map_err(|source| {
            CliError::DeleteMemory(format!("could not look up the id: {source}"))
        })?
    {
        return Ok(memory);
    }
    Err(CliError::DeleteMemory(format!(
        "no live memory found with name or id {target:?} (already deleted, or never existed)"
    )))
}

#[cfg(test)]
mod tests {
    //! In-memory, like the other `debug` command tests: they exercise the command's core logic over a
    //! folded log rather than opening a real store. Here that is [`resolve`] — how a target maps to a
    //! live memory — over a `Graph` materialized from a `MemoryStore`. The disk plumbing (opening the
    //! log, appending the tombstone) is the same thin shell `revert` and `events` leave untested; the
    //! `MemoryDeleted` fold that drops the memory is covered by the graph's own tests.
    use super::resolve;
    use zuihitsu::{
        Clock, EventPayload, EventSource, Graph, MemoryId, MemoryName, MemoryStore, Store,
        SystemClock,
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

    #[test]
    fn a_live_memory_resolves_by_name_or_by_id() {
        let id = MemoryId::generate();
        let graph = graph_of(vec![EventPayload::memory_created(
            id,
            MemoryName::new("context/console:lua"),
        )]);
        assert_eq!(resolve(&graph, "context/console:lua").unwrap().id, id);
        assert_eq!(resolve(&graph, &id.0.to_string()).unwrap().id, id);
    }

    #[test]
    fn a_deleted_or_unknown_memory_does_not_resolve() {
        let id = MemoryId::generate();
        // An unknown name is an error, not a panic.
        let live = graph_of(vec![EventPayload::memory_created(
            id,
            MemoryName::new("context/console:lua"),
        )]);
        assert!(resolve(&live, "person/nobody").is_err());
        // Once tombstoned, the target no longer resolves — so a second delete is a clean no-op rather
        // than a duplicate tombstone, by name or by id.
        let deleted = graph_of(vec![
            EventPayload::memory_created(id, MemoryName::new("context/console:lua")),
            EventPayload::memory_deleted(id),
        ]);
        assert!(resolve(&deleted, "context/console:lua").is_err());
        assert!(resolve(&deleted, &id.0.to_string()).is_err());
    }
}
