//! The event-log seam (host crate view).
//!
//! The seam itself — the `Store` trait, `StoreError`, the in-memory backend, and the subscriber
//! fan-out — lives in `zuihitsu-core` so the wasm replica can share it. This module re-exports that
//! surface and adds the durable, file-backed [`SqliteStore`], which needs the host filesystem (WAL,
//! an exclusive lock) and so cannot move to core.

pub use zuihitsu_core::store::{MemoryStore, Store, StoreError, Subscription};

// The subscriber fan-out helper is shared infrastructure, not public API; `SqliteStore` reaches it
// as `crate::store::notify`.
pub(crate) use zuihitsu_core::store::notify;

mod sqlite;

pub use sqlite::SqliteStore;

#[cfg(test)]
mod tests {
    //! The SQLite backend is held to the same seam contract as the in-memory one, by running the
    //! shared `test_support` helpers from core against it, plus the durability properties unique to a
    //! file-backed log (persistence, the exclusive-writer lock, and corruption surfacing).
    use zuihitsu_core::{
        event::EventSource,
        ids::{MemoryId, Seq},
        store::{
            Store,
            test_support::{
                append_is_ordered_and_faithful, append_stamps_the_source, read_from_returns_tail,
                sample_payloads, subscriber_sees_appends, truncate_removes_the_tail,
            },
        },
        time::Timestamp,
    };

    use crate::store::SqliteStore;

    #[test]
    fn append_is_ordered_and_faithful_sqlite() {
        append_is_ordered_and_faithful(&mut SqliteStore::open_in_memory().unwrap());
    }

    #[test]
    fn read_from_returns_tail_sqlite() {
        read_from_returns_tail(&mut SqliteStore::open_in_memory().unwrap());
    }

    #[test]
    fn subscriber_sees_appends_sqlite() {
        subscriber_sees_appends(&mut SqliteStore::open_in_memory().unwrap());
    }

    #[test]
    fn truncate_removes_the_tail_sqlite() {
        truncate_removes_the_tail(&mut SqliteStore::open_in_memory().unwrap());
    }

    #[test]
    fn append_stamps_the_source_sqlite() {
        append_stamps_the_source(&mut SqliteStore::open_in_memory().unwrap());
    }

    /// A log created before the envelope `source` column existed opens cleanly: the open migrates
    /// the table (an `ADD COLUMN` back-filling `Agent`, the historical fallback), the old rows read
    /// back as `Agent`, and a fresh append stamps its own source alongside them. Disk-backed because
    /// the property under guard is the on-file schema of a pre-source log.
    #[test]
    fn a_pre_source_log_migrates_and_reads_as_agent() {
        let path = temp_log_path("presource");

        // A log written by the pre-source schema: the same table minus the `source` column.
        {
            let conn = rusqlite::Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE events (
                     seq         INTEGER PRIMARY KEY,
                     recorded_at INTEGER NOT NULL,
                     type        TEXT    NOT NULL,
                     target_id   TEXT,
                     version     INTEGER NOT NULL,
                     payload     TEXT    NOT NULL
                 );
                 CREATE INDEX idx_events_target ON events(target_id);
                 INSERT INTO events (seq, recorded_at, type, target_id, version, payload)
                 VALUES (1, 1000, 'DescribePassCompleted', NULL,
                         1, '{\"type\":\"DescribePassCompleted\",\"memories\":[]}');",
            )
            .unwrap();
        }

        let mut store = SqliteStore::open(&path).unwrap();
        let replayed = store.read_from(Seq::ZERO).unwrap();
        assert_eq!(replayed.len(), 1);
        assert_eq!(replayed[0].source, EventSource::Agent);

        // A post-migration append stamps its own authority next to the back-filled rows.
        store
            .append(
                Timestamp::from_millis(2_000),
                EventSource::Operator,
                sample_payloads(),
            )
            .unwrap();
        let replayed = store.read_from(Seq::ZERO).unwrap();
        assert_eq!(replayed[0].source, EventSource::Agent);
        assert!(
            replayed[1..]
                .iter()
                .all(|event| event.source == EventSource::Operator)
        );

        cleanup(&path);
    }

    /// The log survives a process boundary: append, drop, reopen, and the events are still there
    /// in order — the property the whole "rebuild from the log" story rests on.
    #[test]
    fn persists_across_reopen() {
        let path =
            std::env::temp_dir().join(format!("zuihitsu-test-{}.sqlite", MemoryId::generate().0));

        {
            let mut store = SqliteStore::open(&path).unwrap();
            store
                .append(
                    Timestamp::from_millis(1_000),
                    EventSource::Agent,
                    sample_payloads(),
                )
                .unwrap();
        }
        {
            let store = SqliteStore::open(&path).unwrap();
            assert_eq!(store.head().unwrap(), Seq(3));
            assert_eq!(store.read_from(Seq::ZERO).unwrap().len(), 3);
        }

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
        let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
    }

    /// One log, one writer: a second open of the same file is refused while the first is held,
    /// and succeeds once it is released.
    #[test]
    fn exclusive_lock_blocks_a_second_writer() {
        let path =
            std::env::temp_dir().join(format!("zuihitsu-lock-{}.sqlite", MemoryId::generate().0));

        let first = SqliteStore::open(&path).unwrap();
        assert!(SqliteStore::open(&path).is_err()); // already open
        drop(first);
        assert!(SqliteStore::open(&path).is_ok()); // lock released

        cleanup(&path);
    }

    /// A read-only open reads the committed log while another connection still holds the exclusive
    /// write lock — the property the `events` inspection command relies on to be safe against a
    /// running agent.
    #[test]
    fn read_only_open_reads_while_a_writer_holds_the_lock() {
        let path = temp_log_path("readonly");

        let mut writer = SqliteStore::open(&path).unwrap();
        writer
            .append(
                Timestamp::from_millis(1_000),
                EventSource::Agent,
                sample_payloads(),
            )
            .unwrap();

        // The write lock is still held; a read-only open takes no lock and still sees the committed log.
        let reader = SqliteStore::open_read_only(&path).unwrap();
        assert_eq!(reader.read_from(Seq::ZERO).unwrap().len(), 3);

        cleanup(&path);
    }

    /// A crash mid-batch leaves the log clean: an interrupted, uncommitted transaction contributes
    /// nothing, so a reopened log holds exactly the committed events. This is the atomic-batch
    /// guarantee the append path leans on against partial writes (spec §Storage, §Known
    /// limitations → storage-layer corruption).
    #[test]
    fn an_uncommitted_batch_leaves_the_log_clean() {
        let path = temp_log_path("clean");
        {
            let mut store = SqliteStore::open(&path).unwrap();
            store
                .append(
                    Timestamp::from_millis(1_000),
                    EventSource::Agent,
                    sample_payloads(),
                )
                .unwrap(); // seq 1..=3
        }
        // Simulate a kill between INSERT and COMMIT: a raw connection opens a transaction, writes a
        // partial batch, and is dropped before committing — so SQLite must roll it back.
        {
            let conn = rusqlite::Connection::open(&path).unwrap();
            conn.execute_batch("BEGIN").unwrap();
            conn.execute(
                "INSERT INTO events (seq, recorded_at, type, target_id, version, payload)
                 VALUES (4, 9, 'MemoryDeleted', NULL, 1, '{}')",
                [],
            )
            .unwrap();
            // No COMMIT: dropping the connection rolls the transaction back.
        }
        // The reopened log is exactly the committed batch; the abandoned event is gone.
        {
            let store = SqliteStore::open(&path).unwrap();
            assert_eq!(store.head().unwrap(), Seq(3));
            assert_eq!(store.read_from(Seq::ZERO).unwrap().len(), 3);
        }
        cleanup(&path);
    }

    /// A corrupt log surfaces an error rather than silently returning short or wrong data — the
    /// worst failure for a system that rebuilds from the log would be to read a truncated one as if
    /// it were whole (spec §Known limitations → storage-layer corruption).
    #[test]
    fn a_corrupt_log_is_an_error_not_silent_data() {
        let path = temp_log_path("corrupt");
        {
            let mut store = SqliteStore::open(&path).unwrap();
            store
                .append(
                    Timestamp::from_millis(1_000),
                    EventSource::Agent,
                    sample_payloads(),
                )
                .unwrap();
        }
        // Clobber the SQLite header magic with a torn write at the start of the file.
        {
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
            file.write_all(&[0xFFu8; 32]).unwrap();
        }
        // Opening or reading must error, not hand back a partial or empty log as if it were whole.
        let result = SqliteStore::open(&path)
            .and_then(|store| store.read_from(Seq::ZERO).map(|events| events.len()));
        assert!(result.is_err(), "a corrupt log must surface an error");
        cleanup(&path);
    }

    /// A scratch log path unique to one test.
    fn temp_log_path(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("zuihitsu-{tag}-{}.sqlite", MemoryId::generate().0))
    }

    /// Remove a log file and its WAL/shm sidecars, best-effort.
    fn cleanup(path: &std::path::Path) {
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
        let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
    }
}
