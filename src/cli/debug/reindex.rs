//! The `reindex` command: delete the vector index so the next boot rebuilds it from scratch.
//!
//! Used as a post-upgrade step when the vector schema changes (e.g. the addition of the
//! `EntryContextual` space). The command removes the SQLite vector file and its `-wal`/`-shm`
//! sidecars, following the same pattern as `revert`'s `remove_db`. The next boot's
//! `index_catch_up` rebuilds the index from the event log, embedding every entry and description
//! under the current schema. The agent must be stopped first — the command opens no lock, but a
//! running agent holds the vector file open and a rebuild while it serves would mix old and new
//! vectors.

use std::path::{Path, PathBuf};

use crate::cli::error::CliError;

/// Delete the vector index file and its sidecars so the next boot rebuilds the index from the log.
/// The command requires `--yes` to proceed — without it, it only reports what it would do.
pub(crate) fn reindex(config: &zuihitsu::config::EnvConfig, yes: bool) -> Result<(), CliError> {
    let vectors_path = config.storage.vectors();
    if !vectors_path.exists() {
        tracing::info!(
            "no vector index at {} — nothing to reindex; the next boot builds it fresh",
            vectors_path.display(),
        );
        return Ok(());
    }
    if !yes {
        tracing::warn!(
            "re-run with --yes to confirm deleting the vector index at {} (the next boot \
             rebuilds it from the log)",
            vectors_path.display(),
        );
        return Ok(());
    }
    remove_db(&vectors_path)?;
    tracing::info!(
        "deleted the vector index at {}; the next boot rebuilds it from the log",
        vectors_path.display(),
    );
    Ok(())
}

/// Remove a SQLite database file and its `-wal`/`-shm` sidecars, treating an absent file as
/// success. Mirrors `revert`'s `remove_db` so both commands handle derived-store cleanup the
/// same way.
fn remove_db(path: &Path) -> Result<(), CliError> {
    for suffix in ["", "-wal", "-shm"] {
        let mut target = path.as_os_str().to_owned();
        target.push(suffix);
        let target = PathBuf::from(target);
        match std::fs::remove_file(&target) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(CliError::Reindex(format!(
                    "could not remove {}: {source}",
                    target.display()
                )));
            }
        }
    }
    Ok(())
}
