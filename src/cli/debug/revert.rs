//! The `revert` command: truncate the log past a seq and reset the derived stores so the next boot
//! rebuilds at that point, with a preview of the cut before anything is touched.

use std::{
    collections::BTreeMap,
    io::Write,
    path::{Path, PathBuf},
};

use anstyle::{AnsiColor, Style};
use zuihitsu::{Event, Seq, SqliteStore, Store, config::EnvConfig};

use crate::cli::{
    debug::events::{name_map, write_event},
    error::CliError,
};

/// Revert the agent to a prior event. Opens the log read-write — which fails if the agent holds the
/// write lock, so a running agent is refused — and truncates every event past `to`. The materialized
/// graph and the vector index only roll forward, so they cannot be walked back; instead the graph and
/// vector files are dropped (the next boot replays and re-embeds from the shortened log) and any
/// snapshot past `to` is discarded so `restore_if_stale` cannot copy a future state back. Without
/// `--yes`, it reports what it would do and changes nothing.
pub(crate) fn revert(config: &EnvConfig, to: u64, yes: bool) -> Result<(), CliError> {
    let to = Seq(to);
    let log_path = config.storage.event_log();
    let mut store = SqliteStore::open(&log_path).map_err(|source| {
        CliError::Revert(format!(
            "could not open the event log at {} for writing (is the agent running?): {source}",
            log_path.display()
        ))
    })?;
    let head = store
        .head()
        .map_err(|source| CliError::Revert(format!("could not read the log head: {source}")))?;
    if to >= head {
        return Err(CliError::Revert(format!(
            "seq {} is at or past the current head {}; nothing to revert",
            to.0, head.0
        )));
    }

    // Preview the cut: the window of events around the revert point, with everything after it greyed
    // out and a marked rule between, so it is unmistakable what survives and what is removed — shown
    // before anything is touched, in both the dry run and the confirmed run.
    let events = store
        .read_from(Seq(0))
        .map_err(|source| CliError::Revert(format!("could not read the log: {source}")))?;
    let names = name_map(&events);
    show_revert_preview(&events, &names, to, head);

    if !yes {
        tracing::warn!(
            "re-run with --yes to confirm reverting to seq {} (removes {} events and rebuilds the \
             graph and vector index)",
            to.0,
            head.0 - to.0,
        );
        return Ok(());
    }

    let removed = store
        .truncate_to(to)
        .map_err(|source| CliError::Revert(format!("could not truncate the log: {source}")))?;
    drop(store); // release the write lock before touching the derived files.

    let graph_path = config.storage.graph();
    let vectors_path = config.storage.vectors();
    let snapshot_dir = config.snapshots.effective_dir(&graph_path);
    remove_db(&graph_path)?;
    remove_db(&vectors_path)?;
    let pruned = zuihitsu::snapshot::discard_after(&snapshot_dir, to).map_err(|source| {
        CliError::Revert(format!(
            "could not discard snapshots past the revert point: {source}"
        ))
    })?;

    tracing::info!(
        "reverted to seq {}: removed {removed} event(s) and {} snapshot(s); the next boot rebuilds \
         the graph and re-embeds the vector index from the shortened log",
        to.0,
        pruned.len(),
    );
    Ok(())
}

/// Print the window of events around a revert point — up to `CONTEXT` on each side — to stdout, with
/// the events that would be removed (those after `to`) greyed out, and a marked rule between the kept
/// head and the removed tail. Sharing `write_event` with the `events` listing, so the preview reads
/// the same. The total removed count is named even when the window shows only part of the tail.
fn show_revert_preview(events: &[Event], names: &BTreeMap<String, String>, to: Seq, head: Seq) {
    const CONTEXT: u64 = 8;
    let mut out = anstream::stdout();
    let removed_total = head.0 - to.0;
    let (lo, hi) = (to.0.saturating_sub(CONTEXT), to.0.saturating_add(CONTEXT));

    let mut rule_printed = false;
    let mut shown_removed = 0u64;
    for event in events.iter().filter(|e| e.seq.0 >= lo && e.seq.0 <= hi) {
        let faded = event.seq.0 > to.0;
        if faded && !rule_printed {
            print_revert_rule(&mut out, to, removed_total);
            rule_printed = true;
        }
        let _ = write_event(&mut out, event, names, faded);
        shown_removed += u64::from(faded);
    }
    if !rule_printed {
        print_revert_rule(&mut out, to, removed_total);
    }
    if removed_total > shown_removed {
        let dim = Style::new().dimmed();
        let _ = writeln!(
            out,
            "        {dim}… and {} more removed{dim:#}",
            removed_total - shown_removed
        );
    }
}

/// The marked rule between the kept head and the removed tail in the revert preview.
fn print_revert_rule(out: &mut impl Write, to: Seq, removed_total: u64) {
    let mark = Style::new().fg_color(Some(AnsiColor::Yellow.into())).bold();
    let _ = writeln!(
        out,
        "{mark}      ── revert here · seq {} becomes the new head · {removed_total} event(s) below are removed ──{mark:#}",
        to.0,
    );
}

/// Remove a SQLite database file and its `-wal`/`-shm` sidecars, treating an absent file as success.
/// Dropping the derived graph and vector stores so the next boot rebuilds them from the log.
fn remove_db(path: &Path) -> Result<(), CliError> {
    for suffix in ["", "-wal", "-shm"] {
        let mut target = path.as_os_str().to_owned();
        target.push(suffix);
        let target = PathBuf::from(target);
        match std::fs::remove_file(&target) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(CliError::Revert(format!(
                    "could not remove {}: {source}",
                    target.display()
                )));
            }
        }
    }
    Ok(())
}
