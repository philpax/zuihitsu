//! Opening a graph and the projection schema: the DDL batch, the schema-fingerprint guard that
//! resets a graph written under another build's schema (the derived store rebuilds from the log at
//! the next materialisation), and the open paths. The fingerprint is a digest of the DDL itself,
//! so any schema edit moves the stamp with no manually-bumped version to forget.

use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};

use crate::graph::{Graph, GraphError, backend};

impl Graph {
    /// Open (creating if absent) a file-backed graph.
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Graph, GraphError> {
        Self::init(Connection::open(path).map_err(backend)?)
    }

    /// Open an ephemeral in-memory graph — the no-file-I/O configuration tests use.
    pub fn open_in_memory() -> Result<Graph, GraphError> {
        Self::init(Connection::open_in_memory().map_err(backend)?)
    }

    fn init(conn: Connection) -> Result<Graph, GraphError> {
        conn.execute_batch(Self::SCHEMA_SQL).map_err(backend)?;
        let graph = Graph { conn };
        graph.guard_schema()?;
        Ok(graph)
    }

    /// Reset the graph unless its stored schema fingerprint matches this build's, so a binary whose
    /// projection schema has moved never reads or writes a table shape it did not create (an added
    /// column would otherwise surface as a runtime `no such column` error deep in a read). The graph
    /// is a derived store — `materialize_from` rebuilds a reset graph from the event log — so the
    /// reset trades one full replay for schema correctness and loses no logical state. A graph
    /// without a stamp (fresh, or written by a build predating the stamp) resets too: recreating
    /// empty tables is free, and it is the only safe reading of an unstamped file.
    pub(super) fn guard_schema(&self) -> Result<(), GraphError> {
        let expected = schema_fingerprint();
        let stored: Option<i64> = self
            .conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'schema_fingerprint'",
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(backend)?;
        if stored != Some(expected) {
            // The FTS shadow tables (`memories_fts_*`) drop with their virtual table, so they are
            // excluded from the sweep rather than dropped twice.
            let tables: Vec<String> = self
                .conn
                .prepare(
                    "SELECT name FROM sqlite_master
                     WHERE type = 'table' AND name NOT LIKE 'memories_fts_%'",
                )
                .map_err(backend)?
                .query_map([], |r| r.get(0))
                .map_err(backend)?
                .collect::<Result<_, _>>()
                .map_err(backend)?;
            for table in tables {
                self.conn
                    .execute_batch(&format!("DROP TABLE IF EXISTS \"{table}\""))
                    .map_err(backend)?;
            }
            self.conn.execute_batch(Self::SCHEMA_SQL).map_err(backend)?;
            self.conn
                .execute(
                    "INSERT INTO meta (key, value) VALUES ('schema_fingerprint', ?1)",
                    params![expected],
                )
                .map_err(backend)?;
        }
        Ok(())
    }

    /// The projection schema, one idempotent DDL batch. Also the input to `schema_fingerprint`, so
    /// any edit here — a new column, an index change — moves the stamp `guard_schema` checks, with
    /// no manually-bumped version to forget.
    const SCHEMA_SQL: &'static str = "CREATE TABLE IF NOT EXISTS memories (
                 id          TEXT    PRIMARY KEY,
                 name        TEXT    NOT NULL UNIQUE,
                 description TEXT    NOT NULL DEFAULT '',
                 volatility  TEXT    NOT NULL DEFAULT 'Medium',
                 deleted     INTEGER NOT NULL DEFAULT 0,
                 created_at  INTEGER NOT NULL,
                 class_id    TEXT    NOT NULL DEFAULT '',
                 -- Whether the operator has pinned this stub as its `same_as` class's primary. When any
                 -- member of a component carries the flag, recompute_classes resolves the class id to
                 -- the earliest-ULID designated member rather than the earliest member overall.
                 designated_primary INTEGER NOT NULL DEFAULT 0,
                 -- The describer's per-memory watermarks: the seq of the memory's latest content
                 -- change, and the seq of the describer pass that last considered it. A memory is
                 -- stale — needs (re)describing — exactly while last_content_seq > last_described_seq.
                 -- Both are derived from the log, so the describe backlog survives a restart.
                 last_content_seq   INTEGER NOT NULL DEFAULT 0,
                 last_described_seq INTEGER NOT NULL DEFAULT 0
             );
             CREATE INDEX IF NOT EXISTS idx_memories_stale
                 ON memories(last_content_seq, last_described_seq);
             CREATE INDEX IF NOT EXISTS idx_memories_class ON memories(class_id);
             CREATE TABLE IF NOT EXISTS content_entries (
                 entry_id      TEXT    PRIMARY KEY,
                 memory_id     TEXT    NOT NULL,
                 asserted_at   INTEGER NOT NULL,
                 occurred_at   TEXT,
                 occurred_sort INTEGER,
                 occurred_lo   INTEGER,
                 occurred_hi   INTEGER,
                 -- Whether this entry's occurrence was authored at append (the agent stamped
                 -- occurred_at) rather than inferred later by the turn-end temporal extraction. Authored
                 -- is ground truth; extracted is a guess. Representative-date projections prefer an
                 -- authored occurrence so a wrong extracted date never shadows a stated one.
                 occurred_authored INTEGER NOT NULL DEFAULT 0,
                 -- Whether this entry is a mirror of its memory's description (the seed entry
                 -- `memory.create` appends from its `description` argument) rather than an account of a
                 -- real occurrence. A description mirror names no time, so the turn-end temporal
                 -- extraction skips it (see `untimed_entries_since`): timing it would fabricate the
                 -- conversation's own now and collide with a later, correctly-dated append on the memory.
                 description_mirror INTEGER NOT NULL DEFAULT 0,
                 fired_at      INTEGER,
                 surfaced_at   INTEGER,
                 text          TEXT    NOT NULL,
                 told_by       TEXT    NOT NULL,
                 told_in       TEXT,
                 visibility    TEXT    NOT NULL,
                 superseded_by TEXT,
                 -- The stated reason an entry was retracted (`EntryRetracted`), or NULL for a live or
                 -- plainly-superseded entry. A retraction tombstones the entry by stamping
                 -- superseded_by with the entry's own id (so every `superseded_by IS NULL` live filter
                 -- hides it with no extra predicate) and records why here, which the history reads
                 -- surface. A non-NULL retracted_reason is what tells a retraction apart from a
                 -- supersession, whose superseded_by names a distinct successor entry.
                 retracted_reason TEXT,
                 -- The platform a connector-maintained entry belongs to, or NULL for an ordinary
                 -- recorded entry. Projected from the recording event's source: a connector-projected
                 -- participant attribute (username, display name, nickname) carries its platform here,
                 -- so a reader — and the maintenance cleanup passes, which must never mutate a
                 -- connector-owned entry — can tell it apart from an agent-recorded fact.
                 origin_platform TEXT,
                 seq           INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_entries_memory ON content_entries(memory_id);
             CREATE INDEX IF NOT EXISTS idx_entries_occurred_sort
                 ON content_entries(occurred_sort);
             CREATE INDEX IF NOT EXISTS idx_entries_occurred_lo_hi
                 ON content_entries(occurred_lo, occurred_hi);
             CREATE INDEX IF NOT EXISTS idx_entries_pending_wakeup
                 ON content_entries(occurred_sort)
                 WHERE fired_at IS NOT NULL AND surfaced_at IS NULL;
             CREATE TABLE IF NOT EXISTS entry_attestations (
                 entry_id         TEXT    NOT NULL,
                 -- The teller who stands behind the fact, stored as serde JSON (the same encoding
                 -- content_entries.told_by uses), so the composite key ranges over the teller value.
                 teller           TEXT    NOT NULL,
                 told_in          TEXT,
                 asserted_at      INTEGER NOT NULL,
                 -- The attester's own audience posture (serde JSON of Visibility). At or narrower than
                 -- the entry's founding posture by the audience-widening invariant, which the write
                 -- path enforces; the fold trusts the recorded event and never rejects here.
                 posture          TEXT    NOT NULL,
                 -- The attester's own wording, when it differed from the entry text (history/console
                 -- only), or NULL when the attestation added no distinct phrasing.
                 phrasing         TEXT,
                 -- The retired entry a consolidation carried this attestation from, or NULL for a
                 -- direct endorsement.
                 source_entry     TEXT,
                 -- The stated reason this attestation was withdrawn (`AttestationRetracted`), or NULL
                 -- for a live attestation. A whole-entry retraction stamps every live attestation's
                 -- reason so history stays coherent.
                 retracted_reason TEXT,
                 seq              INTEGER NOT NULL,
                 -- Identity is the (entry, teller) pair: a re-attestation by the same teller is
                 -- last-writer-wins on the row, and the founding attestation is the teller the entry's
                 -- own MemoryContentAppended recorded.
                 PRIMARY KEY (entry_id, teller)
             );
             CREATE INDEX IF NOT EXISTS idx_entry_attestations_entry
                 ON entry_attestations(entry_id, seq);
             CREATE TABLE IF NOT EXISTS entry_disputes (
                 entry_id  TEXT PRIMARY KEY,
                 memory_id TEXT NOT NULL,
                 statement TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_entry_disputes_memory
                 ON entry_disputes(memory_id);
             CREATE TABLE IF NOT EXISTS tags (
                 name        TEXT PRIMARY KEY,
                 description TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS memory_tags (
                 memory_id TEXT NOT NULL,
                 tag       TEXT NOT NULL,
                 PRIMARY KEY (memory_id, tag)
             );
             CREATE TABLE IF NOT EXISTS relations (
                 name        TEXT    PRIMARY KEY,
                 inverse     TEXT    NOT NULL,
                 from_card   TEXT    NOT NULL,
                 to_card     TEXT    NOT NULL,
                 symmetric   INTEGER NOT NULL,
                 reflexive   INTEGER NOT NULL,
                 description TEXT    NOT NULL DEFAULT ''
             );
             CREATE TABLE IF NOT EXISTS links (
                 from_id    TEXT NOT NULL,
                 to_id      TEXT NOT NULL,
                 relation   TEXT NOT NULL,
                 source     TEXT NOT NULL,
                 told_by    TEXT,
                 told_in    TEXT,
                 visibility TEXT NOT NULL DEFAULT 'Public',
                 PRIMARY KEY (from_id, to_id, relation)
             );
             CREATE INDEX IF NOT EXISTS idx_links_to ON links(to_id, relation);
             CREATE TABLE IF NOT EXISTS memory_aliases (
                 former_name TEXT PRIMARY KEY,
                 memory_id   TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS conversations (
                 id             TEXT    PRIMARY KEY,
                 platform       TEXT    NOT NULL,
                 scope_path     TEXT    NOT NULL,
                 context_memory TEXT    NOT NULL,
                 ended          INTEGER NOT NULL DEFAULT 0
             );
             CREATE UNIQUE INDEX IF NOT EXISTS idx_conversations_locator
                 ON conversations(platform, scope_path);
             CREATE TABLE IF NOT EXISTS sessions (
                 id               TEXT    PRIMARY KEY,
                 conversation     TEXT    NOT NULL,
                 started_at       INTEGER NOT NULL,
                 seeded_from_turn TEXT,
                 brief            TEXT    NOT NULL,
                 ended            INTEGER NOT NULL DEFAULT 0,
                 end_cause        TEXT,
                 seq              INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_sessions_conversation ON sessions(conversation);
             CREATE TABLE IF NOT EXISTS session_participants (
                 session TEXT NOT NULL,
                 memory  TEXT NOT NULL,
                 at_turn TEXT,
                 PRIMARY KEY (session, memory)
             );
             CREATE TABLE IF NOT EXISTS participant_identities (
                 platform         TEXT NOT NULL,
                 platform_user_id TEXT NOT NULL,
                 memory           TEXT NOT NULL,
                 PRIMARY KEY (platform, platform_user_id)
             );
             CREATE INDEX IF NOT EXISTS idx_participant_identities_memory
                 ON participant_identities(memory);
             CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value INTEGER NOT NULL);
             CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(
                 name, description, content, memory_id UNINDEXED
             );";
}

/// The stamp `guard_schema` compares: the leading eight bytes of a SHA-256 over the schema DDL,
/// stored in `meta` as an integer. A digest of the DDL text itself, so the stamp is a pure function
/// of the schema with no versioning discipline to uphold.
pub(super) fn schema_fingerprint() -> i64 {
    let digest = Sha256::digest(Graph::SCHEMA_SQL.as_bytes());
    i64::from_le_bytes(
        digest[..8]
            .try_into()
            .expect("a SHA-256 digest holds at least eight bytes"),
    )
}
