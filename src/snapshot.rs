//! Graph snapshots: `VACUUM INTO` checkpoints of the derived graph, restored on boot to skip a full
//! replay of the log from `seq 0` (spec §Snapshots).
//!
//! The log is the source of truth and is always retained in full, so there is nothing to snapshot
//! there; what is expensive to rebuild is the graph projection. A snapshot is a complete graph
//! database file tagged with the log `seq` it captured (its `graph_head`), produced by
//! [`Graph::snapshot_into`](crate::graph::Graph::snapshot_into). This module owns the file naming and
//! the boot-time restore decision; the act of writing one lives on `Graph`, and scheduling them lives
//! on the server.

use std::{
    fs,
    path::{Path, PathBuf},
};

use rusqlite::{Connection, OptionalExtension};

use crate::ids::Seq;

/// The snapshot filename for a graph captured at `head`. Zero-padded so a lexical sort of the
/// directory is a numeric sort by `seq` — [`latest`] relies on this.
pub fn snapshot_filename(head: Seq) -> String {
    format!("snapshot-{:020}.sqlite", head.0)
}

/// The `seq` a snapshot file was captured at, parsed from its name, or `None` if the name is not a
/// snapshot file. The name carries the head (spec §Snapshots → "tagged with the log `seq`").
pub fn parse_snapshot_head(name: &str) -> Option<Seq> {
    let digits = name.strip_prefix("snapshot-")?.strip_suffix(".sqlite")?;
    digits.parse::<u64>().ok().map(Seq)
}

/// The newest snapshot in `dir` — the file with the highest captured head — or `None` if the
/// directory holds none (or does not exist). Used by both the boot restore and retention pruning.
pub fn latest(dir: &Path) -> Result<Option<(PathBuf, Seq)>, SnapshotError> {
    Ok(snapshots(dir)?.into_iter().max_by_key(|(_, head)| head.0))
}

/// Every snapshot in `dir`, oldest first (by captured head). Returns an empty list when the directory
/// is absent, so a never-snapshotted instance is not an error.
pub fn snapshots(dir: &Path) -> Result<Vec<(PathBuf, Seq)>, SnapshotError> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut found = Vec::new();
    for entry in fs::read_dir(dir).map_err(|source| SnapshotError::ReadDir {
        path: dir.to_path_buf(),
        source,
    })? {
        let entry = entry.map_err(|source| SnapshotError::ReadDir {
            path: dir.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        if let Some(name) = path.file_name().and_then(|name| name.to_str())
            && let Some(head) = parse_snapshot_head(name)
        {
            found.push((path, head));
        }
    }
    found.sort_by_key(|(_, head)| head.0);
    Ok(found)
}

/// Restore the graph at `graph_path` from the latest snapshot in `dir` when the live graph is behind
/// it — a fresh, deleted, or corrupt graph whose head is below the newest checkpoint. Copies the
/// snapshot over the graph path (so materialization then replays only the tail from the snapshot's
/// head, not the whole log). A no-op, returning `None`, when there is no snapshot or the live graph is
/// already at or ahead of it (the steady state, where the persisted graph leads its checkpoints).
/// Returns the restored head when a copy happened. Must run before the graph file is opened.
pub fn restore_if_stale(graph_path: &Path, dir: &Path) -> Result<Option<Seq>, SnapshotError> {
    let Some((snapshot_path, snapshot_head)) = latest(dir)? else {
        return Ok(None);
    };
    if snapshot_head <= read_graph_head(graph_path)? {
        return Ok(None);
    }
    // The graph is a WAL database, so a killed process can leave `-wal`/`-shm` siblings that
    // shadow the main file: copying a (self-contained, checkpointed) snapshot over `graph_path`
    // while a stale `graph.sqlite-wal` still sits beside it would let the live WAL replay over the
    // fresh copy and resurrect the pre-restore state. Clear the siblings first so the copy lands on
    // a clean target. `VACUUM INTO` snapshots carry no WAL of their own.
    for suffix in ["-wal", "-shm"] {
        let mut sibling = graph_path.as_os_str().to_owned();
        sibling.push(suffix);
        let _ = fs::remove_file(PathBuf::from(sibling));
    }
    fs::copy(&snapshot_path, graph_path).map_err(|source| SnapshotError::Restore {
        from: snapshot_path,
        to: graph_path.to_path_buf(),
        source,
    })?;
    Ok(Some(snapshot_head))
}

/// Keep the `keep` newest snapshots in `dir` and delete the rest, returning the paths removed. A
/// `keep` of 0 is treated as 1: retention never deletes the only checkpoint boot would restore from.
pub fn prune(dir: &Path, keep: usize) -> Result<Vec<PathBuf>, SnapshotError> {
    let keep = keep.max(1);
    let mut all = snapshots(dir)?; // oldest first
    if all.len() <= keep {
        return Ok(Vec::new());
    }
    let surplus = all.len() - keep;
    let mut removed = Vec::new();
    for (path, _head) in all.drain(..surplus) {
        fs::remove_file(&path).map_err(|source| SnapshotError::Prune {
            path: path.clone(),
            source,
        })?;
        removed.push(path);
    }
    Ok(removed)
}

/// Delete every snapshot in `dir` whose head is past `to`, returning the paths removed. The revert path
/// uses this: a checkpoint captured after the revert point describes a state the truncated log no longer
/// reaches, and `restore_if_stale` would otherwise copy it back over the rebuilt graph and undo the
/// revert. Checkpoints at or before `to` are kept — they still speed the rebuild to that point.
pub fn discard_after(dir: &Path, to: Seq) -> Result<Vec<PathBuf>, SnapshotError> {
    let mut removed = Vec::new();
    for (path, head) in snapshots(dir)? {
        if head > to {
            fs::remove_file(&path).map_err(|source| SnapshotError::Prune {
                path: path.clone(),
                source,
            })?;
            removed.push(path);
        }
    }
    Ok(removed)
}

/// The `graph_head` recorded in the graph file at `path`, or `Seq::ZERO` when the file is absent or
/// has no head yet (an empty or never-materialized graph) — the baseline the restore decision
/// compares a snapshot against.
pub fn read_graph_head(path: &Path) -> Result<Seq, SnapshotError> {
    if !path.exists() {
        return Ok(Seq::ZERO);
    }
    let conn = Connection::open(path).map_err(|source| SnapshotError::OpenGraph {
        path: path.to_path_buf(),
        source,
    })?;
    // Read-only at its core but opened with the read-write flag so SQLite can recover a WAL left
    // hot by a killed process — the head a restore decision compares must see every committed event,
    // including one only present in the live WAL. The shared pragma set (busy timeout, NORMAL sync,
    // FKs on) applies; journal mode is already what the file is.
    crate::db::sqlite_defaults(&conn, false).map_err(|source| SnapshotError::OpenGraph {
        path: path.to_path_buf(),
        source,
    })?;
    let head: Option<i64> = conn
        .query_row(
            "SELECT value FROM meta WHERE key = 'graph_head'",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(|source| SnapshotError::OpenGraph {
            path: path.to_path_buf(),
            source,
        })?;
    Ok(Seq(head.unwrap_or(0) as u64))
}

/// A failure managing snapshot files — directory scanning, reading a graph's head, or the restore
/// copy. The `Display` leads with the `snapshot:` subsystem context (spec §Error handling).
#[derive(Debug)]
pub enum SnapshotError {
    ReadDir {
        path: PathBuf,
        source: std::io::Error,
    },
    OpenGraph {
        path: PathBuf,
        source: rusqlite::Error,
    },
    Restore {
        from: PathBuf,
        to: PathBuf,
        source: std::io::Error,
    },
    Prune {
        path: PathBuf,
        source: std::io::Error,
    },
}

impl std::fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SnapshotError::ReadDir { path, source } => {
                write!(
                    f,
                    "snapshot: could not read the snapshot directory {path:?}: {source}"
                )
            }
            SnapshotError::OpenGraph { path, source } => {
                write!(
                    f,
                    "snapshot: could not read the graph head at {path:?}: {source}"
                )
            }
            SnapshotError::Restore { from, to, source } => write!(
                f,
                "snapshot: could not restore the graph from {from:?} to {to:?}: {source}"
            ),
            SnapshotError::Prune { path, source } => {
                write!(
                    f,
                    "snapshot: could not prune the old snapshot {path:?}: {source}"
                )
            }
        }
    }
}

impl std::error::Error for SnapshotError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SnapshotError::ReadDir { source, .. } => Some(source),
            SnapshotError::OpenGraph { source, .. } => Some(source),
            SnapshotError::Restore { source, .. } => Some(source),
            SnapshotError::Prune { source, .. } => Some(source),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        discard_after, latest, parse_snapshot_head, prune, read_graph_head, restore_if_stale,
        snapshot_filename, snapshots,
    };
    use crate::ids::Seq;
    use rusqlite::Connection;
    use std::{
        fs,
        path::{Path, PathBuf},
    };

    /// A unique scratch directory for one test, cleaned up by the test that made it.
    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("zuihitsu-snapshot-{}", ulid::Ulid::new()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A minimal graph file carrying `head` in its `meta` table — enough for the restore decision,
    /// without standing up the full graph schema.
    fn write_graph_head(path: &Path, head: u64) {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value INTEGER NOT NULL);",
        )
        .unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO meta (key, value) VALUES ('graph_head', ?1)",
            [head as i64],
        )
        .unwrap();
    }

    #[test]
    fn filename_and_head_round_trip() {
        assert_eq!(
            snapshot_filename(Seq(42)),
            "snapshot-00000000000000000042.sqlite"
        );
        assert_eq!(
            parse_snapshot_head(&snapshot_filename(Seq(42))),
            Some(Seq(42))
        );
        // Zero-padding makes a lexical sort a numeric sort.
        assert!(snapshot_filename(Seq(9)) < snapshot_filename(Seq(10)));
        // Non-snapshot names are ignored.
        assert_eq!(parse_snapshot_head("graph.sqlite"), None);
        assert_eq!(parse_snapshot_head("snapshot-oops.sqlite"), None);
    }

    #[test]
    fn latest_picks_the_highest_head_and_tolerates_an_absent_dir() {
        let dir = temp_dir();
        let snaps = dir.join("does-not-exist");
        assert!(latest(&snaps).unwrap().is_none());

        fs::create_dir_all(&snaps).unwrap();
        for head in [3u64, 10, 7] {
            write_graph_head(&snaps.join(snapshot_filename(Seq(head))), head);
        }
        let (path, head) = latest(&snaps).unwrap().unwrap();
        assert_eq!(head, Seq(10));
        assert_eq!(path, snaps.join(snapshot_filename(Seq(10))));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn read_graph_head_handles_absent_and_present() {
        let dir = temp_dir();
        assert_eq!(
            read_graph_head(&dir.join("nope.sqlite")).unwrap(),
            Seq::ZERO
        );
        let graph = dir.join("graph.sqlite");
        write_graph_head(&graph, 17);
        assert_eq!(read_graph_head(&graph).unwrap(), Seq(17));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn restore_copies_a_newer_snapshot_over_a_stale_or_missing_graph() {
        let dir = temp_dir();
        let snaps = dir.join("snaps");
        fs::create_dir_all(&snaps).unwrap();
        write_graph_head(&snaps.join(snapshot_filename(Seq(20))), 20);

        // A graph behind the latest snapshot is restored from it.
        let stale = dir.join("stale.sqlite");
        write_graph_head(&stale, 5);
        assert_eq!(restore_if_stale(&stale, &snaps).unwrap(), Some(Seq(20)));
        assert_eq!(read_graph_head(&stale).unwrap(), Seq(20));

        // A missing graph is restored too (the rebuild-from-snapshot path).
        let missing = dir.join("missing.sqlite");
        assert_eq!(restore_if_stale(&missing, &snaps).unwrap(), Some(Seq(20)));
        assert_eq!(read_graph_head(&missing).unwrap(), Seq(20));
        fs::remove_dir_all(&dir).unwrap();
    }

    /// Restore must clear a killed process's WAL siblings before copying the snapshot over the main
    /// file. The graph is a WAL database, so a stale `-wal`/`-shm` beside a fresh `VACUUM INTO` copy
    /// would let the live WAL replay over it and resurrect the pre-restore state (spec §Snapshots).
    #[test]
    fn restore_clears_wal_siblings_before_copying() {
        let dir = temp_dir();
        let snaps = dir.join("snaps");
        fs::create_dir_all(&snaps).unwrap();
        write_graph_head(&snaps.join(snapshot_filename(Seq(20))), 20);

        let stale = dir.join("stale.sqlite");
        write_graph_head(&stale, 5);
        // A killed process left a live WAL and its shared-memory index beside the graph.
        fs::write(dir.join("stale.sqlite-wal"), b"stale wal").unwrap();
        fs::write(dir.join("stale.sqlite-shm"), b"stale shm").unwrap();

        assert_eq!(restore_if_stale(&stale, &snaps).unwrap(), Some(Seq(20)));
        // The siblings are gone, so the copy cannot be shadowed by them.
        assert!(!dir.join("stale.sqlite-wal").exists());
        assert!(!dir.join("stale.sqlite-shm").exists());
        assert_eq!(read_graph_head(&stale).unwrap(), Seq(20));
        fs::remove_dir_all(&dir).unwrap();
    }

    /// `read_graph_head` reads a graph whose committed head lives only in a hot WAL: a connection
    /// which cannot recover the WAL (a read-only one) would see a stale or empty head, and the
    /// restore decision would wrongly think the snapshot leads the graph. The open is read-write so
    /// SQLite recovers the WAL, and the head it reports includes every committed event.
    #[test]
    fn read_graph_head_sees_a_hot_wal() {
        let dir = temp_dir();
        let graph = dir.join("graph.sqlite");
        let conn = Connection::open(&graph).unwrap();
        conn.pragma_update(None, "journal_mode", "WAL").unwrap();
        conn.execute_batch("CREATE TABLE meta (key TEXT PRIMARY KEY, value INTEGER NOT NULL);")
            .unwrap();
        // Commit into the WAL without a checkpoint: the head exists only in the live `-wal` file.
        conn.execute(
            "INSERT OR REPLACE INTO meta (key, value) VALUES ('graph_head', 42)",
            [],
        )
        .unwrap();
        // The connection is still open, so the WAL is hot and un-checkpointed.
        assert_eq!(read_graph_head(&graph).unwrap(), Seq(42));
        assert!(
            dir.join("graph.sqlite-wal").exists(),
            "the WAL must still be live when the head read succeeds"
        );
        drop(conn);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn prune_keeps_the_newest_and_never_the_only_one() {
        let dir = temp_dir();
        for head in [5u64, 10, 15, 20, 25] {
            write_graph_head(&dir.join(snapshot_filename(Seq(head))), head);
        }
        // Keep the 2 newest: the 3 oldest are removed.
        let removed = prune(&dir, 2).unwrap();
        assert_eq!(removed.len(), 3);
        let kept: Vec<u64> = snapshots(&dir).unwrap().iter().map(|(_, h)| h.0).collect();
        assert_eq!(kept, vec![20, 25]);

        // keep = 0 is treated as 1, so the latest checkpoint is never deleted.
        prune(&dir, 0).unwrap();
        assert_eq!(latest(&dir).unwrap().unwrap().1, Seq(25));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn discard_after_drops_snapshots_past_the_revert_point() {
        let dir = temp_dir();
        for head in [5u64, 10, 15, 20, 25] {
            write_graph_head(&dir.join(snapshot_filename(Seq(head))), head);
        }
        // Reverting to seq 12 leaves the snapshots at 15, 20, and 25 describing a state the truncated
        // log no longer reaches; they are removed, and the 5 and 10 checkpoints are kept.
        let removed = discard_after(&dir, Seq(12)).unwrap();
        assert_eq!(removed.len(), 3);
        let kept: Vec<u64> = snapshots(&dir).unwrap().iter().map(|(_, h)| h.0).collect();
        assert_eq!(kept, vec![5, 10]);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn restore_is_a_no_op_when_the_graph_leads_its_snapshots() {
        let dir = temp_dir();
        let snaps = dir.join("snaps");
        fs::create_dir_all(&snaps).unwrap();
        write_graph_head(&snaps.join(snapshot_filename(Seq(20))), 20);
        let graph = dir.join("graph.sqlite");
        write_graph_head(&graph, 30); // ahead of the snapshot — the steady state

        assert_eq!(restore_if_stale(&graph, &snaps).unwrap(), None);
        assert_eq!(read_graph_head(&graph).unwrap(), Seq(30)); // untouched
        fs::remove_dir_all(&dir).unwrap();
    }
}
