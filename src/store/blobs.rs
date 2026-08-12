//! The content-addressed blob store: the bytes of attachments the event log refers to only by
//! [`BlobHash`]. Separate from the log because the log is the source of truth and stays small and
//! replayable, while blob bytes are bulk, immutable, and rebuildable-from-nowhere — so they live in
//! their own SQLite database with their own lifecycle (a later GC sweep reconciles them against the
//! hashes the log still mentions).
//!
//! The hash *is* the key, so a write with the same bytes and MIME is idempotent: putting the same
//! content twice stores one blob and yields one address, while a conflicting MIME is rejected.

use std::{path::Path, sync::Arc};

use rusqlite::{Connection, OpenFlags, OptionalExtension, params};

use crate::{
    clock::{Clock, SystemClock},
    db::{self, query_map_into, query_opt_into},
    ids::BlobHash,
};

/// A stored blob: its bytes and the media type they were stored under.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Blob {
    pub bytes: Vec<u8>,
    pub mime: String,
}

/// A stored blob's metadata, readable without loading its bytes — what a `HEAD` request, a size
/// check, or a GC sweep needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlobMeta {
    pub mime: String,
    pub byte_len: u64,
}

/// A blob-store failure: a backend error, a MIME conflict, or a stored row whose recorded length does
/// not fit the representable range.
#[derive(Debug)]
pub enum BlobError {
    Backend(String),
    /// The content address already exists under a different media type. Existing metadata is never
    /// changed because recorded attachments rely on it remaining stable for replay.
    MimeConflict {
        hash: BlobHash,
        existing_mime: String,
        requested_mime: String,
    },
    Malformed(String),
}

impl std::fmt::Display for BlobError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BlobError::Backend(message) => write!(f, "blob store: {message}"),
            BlobError::MimeConflict {
                hash,
                existing_mime,
                requested_mime,
            } => write!(
                f,
                "blob store: content address {hash} is stored as {existing_mime}, not {requested_mime}"
            ),
            BlobError::Malformed(message) => write!(f, "blob store: {message}"),
        }
    }
}

impl std::error::Error for BlobError {}

impl From<rusqlite::Error> for BlobError {
    fn from(error: rusqlite::Error) -> BlobError {
        BlobError::Backend(error.to_string())
    }
}

/// The SQLite-backed blob store. One `blobs` table keyed by content address, written once per
/// distinct content and never modified.
pub struct BlobStore {
    conn: Connection,
    clock: Arc<dyn Clock>,
}

impl BlobStore {
    /// Open (creating if absent) a file-backed store at `path`, in WAL mode.
    pub fn open(path: impl AsRef<Path>) -> Result<BlobStore, BlobError> {
        let conn = Connection::open(path)?;
        db::sqlite_defaults(&conn, true)?;
        BlobStore::init(conn)
    }

    /// Open an existing store at `path` for reading only — the read-only serving mode, which creates
    /// nothing on disk and writes nothing. The file must already exist; a missing one is an error
    /// naming it, since a read-only boot has no business creating an instance's storage.
    pub fn open_read_only(path: impl AsRef<Path>) -> Result<BlobStore, BlobError> {
        let conn = Connection::open_with_flags(
            path.as_ref(),
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        db::sqlite_defaults(&conn, false)?;
        Ok(BlobStore {
            conn,
            clock: Arc::new(SystemClock),
        })
    }

    /// Open an ephemeral in-memory store — what the tests use.
    pub fn open_in_memory() -> Result<BlobStore, BlobError> {
        let conn = Connection::open_in_memory()?;
        db::sqlite_defaults(&conn, false)?;
        BlobStore::init(conn)
    }

    /// Read `created_at` from an injected clock instead of the system one, so a test that turns on
    /// blob age (a retention sweep) drives time explicitly.
    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> BlobStore {
        self.clock = clock;
        self
    }

    /// Store `bytes` under their content address, returning it. Re-putting identical bytes is
    /// idempotent only when the requested media type matches the stored one; a conflicting type is
    /// rejected so historical attachment metadata cannot change.
    pub fn put(&self, bytes: &[u8], mime: &str) -> Result<BlobHash, BlobError> {
        let hash = BlobHash::of(bytes);
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| {
            let existing_mime: Option<String> = self
                .conn
                .query_row(
                    "SELECT mime FROM blobs WHERE hash = ?1",
                    params![hash.as_str()],
                    |row| row.get(0),
                )
                .optional()?;
            if let Some(existing_mime) = existing_mime {
                if existing_mime != mime {
                    return Err(BlobError::MimeConflict {
                        hash: hash.clone(),
                        existing_mime,
                        requested_mime: mime.to_owned(),
                    });
                }
                return Ok(hash.clone());
            }

            self.conn.execute(
                "INSERT INTO blobs (hash, mime, byte_len, created_at, bytes) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    hash.as_str(),
                    mime,
                    bytes.len() as i64,
                    self.clock.now().as_millisecond(),
                    bytes,
                ],
            )?;
            Ok(hash.clone())
        })();
        match result {
            Ok(hash) => {
                self.conn.execute_batch("COMMIT")?;
                Ok(hash)
            }
            Err(error) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(error)
            }
        }
    }

    /// The blob stored under `hash`, or `None` when the store holds none.
    pub fn get(&self, hash: &BlobHash) -> Result<Option<Blob>, BlobError> {
        let stmt = self
            .conn
            .prepare("SELECT bytes, mime FROM blobs WHERE hash = ?1")?;
        query_opt_into(stmt, params![hash.as_str()], |row| {
            let (bytes, mime): (Vec<u8>, String) = row.try_into()?;
            Ok(Blob { bytes, mime })
        })
    }

    /// The metadata of the blob stored under `hash`, without loading its bytes.
    pub fn head(&self, hash: &BlobHash) -> Result<Option<BlobMeta>, BlobError> {
        let stmt = self
            .conn
            .prepare("SELECT mime, byte_len FROM blobs WHERE hash = ?1")?;
        query_opt_into(stmt, params![hash.as_str()], |row| {
            let (mime, byte_len): (String, i64) = row.try_into()?;
            Ok(BlobMeta {
                mime,
                byte_len: u64::try_from(byte_len).map_err(|_| {
                    BlobError::Malformed(format!(
                        "blob {hash} records a negative length {byte_len}"
                    ))
                })?,
            })
        })
    }

    /// Every stored content address, for a GC sweep to reconcile against the log.
    pub fn hashes(&self) -> Result<Vec<BlobHash>, BlobError> {
        let stmt = self.conn.prepare("SELECT hash FROM blobs ORDER BY hash")?;
        query_map_into(stmt, [], |row| {
            let hash: String = row.get(0)?;
            hash.parse()
                .map_err(|_| BlobError::Malformed(format!("stored hash {hash} is not an address")))
        })
    }

    /// Drop the blob stored under `hash`, reporting whether one was there — the GC sweep's write.
    pub fn remove(&self, hash: &BlobHash) -> Result<bool, BlobError> {
        let removed = self
            .conn
            .execute("DELETE FROM blobs WHERE hash = ?1", params![hash.as_str()])?;
        Ok(removed > 0)
    }

    fn init(conn: Connection) -> Result<BlobStore, BlobError> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS blobs (
                 hash       TEXT    PRIMARY KEY,
                 mime       TEXT    NOT NULL,
                 byte_len   INTEGER NOT NULL,
                 created_at INTEGER NOT NULL,
                 bytes      BLOB    NOT NULL
             ) STRICT;",
        )?;
        Ok(BlobStore {
            conn,
            clock: Arc::new(SystemClock),
        })
    }
}

#[cfg(test)]
mod tests {
    //! The blob store's storage contract: content addressing (so a write is idempotent), the
    //! bytes-free metadata read, the GC sweep's listing and removal, and the address parser's
    //! rejection of anything that is not 64 lowercase hex characters.

    use crate::{
        ids::BlobHash,
        store::{BlobMeta, BlobStore},
    };

    const PNG: &str = "image/png";

    #[test]
    fn blob_round_trips_through_put_and_get() {
        let store = BlobStore::open_in_memory().unwrap();
        let hash = store.put(b"the bytes of an attachment", PNG).unwrap();

        let blob = store.get(&hash).unwrap().expect("the blob just put");
        assert_eq!(blob.bytes, b"the bytes of an attachment");
        assert_eq!(blob.mime, PNG);
        assert_eq!(hash, BlobHash::of(b"the bytes of an attachment"));
    }

    #[test]
    fn put_is_idempotent_for_same_content_and_mime() {
        let store = BlobStore::open_in_memory().unwrap();
        let first = store.put(b"same bytes", PNG).unwrap();
        let second = store.put(b"same bytes", PNG).unwrap();
        assert_eq!(first, second);
        assert_eq!(store.hashes().unwrap(), vec![first]);

        // Different bytes are a different blob, so the store now holds two.
        store.put(b"other bytes", PNG).unwrap();
        assert_eq!(store.hashes().unwrap().len(), 2);
    }

    #[test]
    fn put_rejects_same_content_with_different_mime() {
        let store = BlobStore::open_in_memory().unwrap();
        let hash = store.put(b"same bytes", PNG).unwrap();
        let error = store.put(b"same bytes", "text/plain").unwrap_err();
        assert!(matches!(
            error,
            super::BlobError::MimeConflict {
                hash: conflict_hash,
                existing_mime,
                requested_mime,
            } if conflict_hash == hash && existing_mime == PNG && requested_mime == "text/plain"
        ));
        assert_eq!(store.head(&hash).unwrap().unwrap().mime, PNG);
    }

    #[test]
    fn concurrent_file_backed_puts_have_one_success_and_one_mime_conflict() {
        let path =
            std::env::temp_dir().join(format!("zuihitsu-blobs-{}.sqlite", BlobHash::of(b"race")));
        let _ = std::fs::remove_file(&path);
        let first = BlobStore::open(&path).unwrap();
        let second = BlobStore::open(&path).unwrap();
        let bytes = b"race bytes".to_vec();
        let expected_hash = BlobHash::of(&bytes);
        let first_bytes = bytes.clone();
        let second_bytes = bytes.clone();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let first_barrier = barrier.clone();
        let second_barrier = barrier.clone();
        let first_thread = std::thread::spawn(move || {
            first_barrier.wait();
            first.put(&first_bytes, "image/png")
        });
        let second_thread = std::thread::spawn(move || {
            second_barrier.wait();
            second.put(&second_bytes, "text/plain")
        });
        let first_result = first_thread.join().unwrap();
        let second_result = second_thread.join().unwrap();
        assert_eq!(first_result.is_ok() as u8 + second_result.is_ok() as u8, 1);
        assert!(
            matches!(first_result, Err(super::BlobError::MimeConflict { .. }))
                || matches!(second_result, Err(super::BlobError::MimeConflict { .. }))
        );
        let reader = BlobStore::open(&path).unwrap();
        let mime = reader.head(&expected_hash).unwrap().unwrap().mime;
        assert!(mime == "image/png" || mime == "text/plain");
        drop(reader);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn an_unknown_hash_reads_as_absent() {
        let store = BlobStore::open_in_memory().unwrap();
        let missing = BlobHash::of(b"never stored");
        assert_eq!(store.get(&missing).unwrap(), None);
        assert_eq!(store.head(&missing).unwrap(), None);
    }

    #[test]
    fn head_reports_the_mime_and_length() {
        let store = BlobStore::open_in_memory().unwrap();
        let hash = store.put(b"0123456789", "application/pdf").unwrap();
        assert_eq!(
            store.head(&hash).unwrap(),
            Some(BlobMeta {
                mime: "application/pdf".to_owned(),
                byte_len: 10,
            })
        );
    }

    #[test]
    fn remove_reports_whether_a_blob_was_there() {
        let store = BlobStore::open_in_memory().unwrap();
        let hash = store.put(b"transient", PNG).unwrap();
        assert!(store.remove(&hash).unwrap());
        assert!(!store.remove(&hash).unwrap());
        assert_eq!(store.get(&hash).unwrap(), None);
        assert!(store.hashes().unwrap().is_empty());
    }

    #[test]
    fn hashes_lists_every_stored_address() {
        let store = BlobStore::open_in_memory().unwrap();
        let mut expected = vec![
            store.put(b"a", PNG).unwrap(),
            store.put(b"b", PNG).unwrap(),
            store.put(b"c", PNG).unwrap(),
        ];
        expected.sort();
        assert_eq!(store.hashes().unwrap(), expected);
    }

    #[test]
    fn an_address_parses_only_as_64_lowercase_hex_characters() {
        let valid = BlobHash::of(b"anything").to_string();
        assert_eq!(valid.len(), 64);
        assert_eq!(valid.parse::<BlobHash>().unwrap().as_str(), valid);

        assert!(valid.to_uppercase().parse::<BlobHash>().is_err());
        assert!(valid[..63].parse::<BlobHash>().is_err());
        assert!(format!("{valid}0").parse::<BlobHash>().is_err());
        assert!("".parse::<BlobHash>().is_err());
        assert!(
            format!("{}zz", &valid[..62]).parse::<BlobHash>().is_err(),
            "non-hex characters must be rejected"
        );
        assert!(
            "../../etc/passwd".parse::<BlobHash>().is_err(),
            "a path-shaped string must never reach the store as an address"
        );
    }
}
