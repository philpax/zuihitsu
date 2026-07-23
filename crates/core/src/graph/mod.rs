//! The materialized graph: a pure projection of the event log into queryable SQLite tables.
//!
//! Derived state — it can be dropped and rebuilt from the log at any time without data loss (spec
//! §Storage). This root holds the schema, the open/boot path, and the shared types and helpers; the
//! [`Graph::apply`] materializer lives in [`apply`], and the agent-facing query methods are split by
//! sub-domain across [`memories`], [`occurrences`], [`entries`], [`vocabulary`], [`links`],
//! [`sessions`], and [`search`].

use rusqlite::{Connection, OptionalExtension, params, types::ValueRef};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use ulid::Ulid;

use crate::{
    db::query_map_into,
    event::{
        Cardinality, ConversationRef, EventSource, LinkSource, Teller, Visibility, Volatility,
    },
    ids::{ConversationId, ConversationLocator, EntryId, MemoryId, MemoryName, Seq, SessionId},
    store::Store,
    time::{TemporalRef, Timestamp},
    vocabulary::{RelationName, TagName},
};

mod apply;
mod describe;
mod entries;
mod error;
mod links;
mod memories;
mod occurrences;
mod search;
mod sessions;
#[cfg(test)]
mod tests;
mod vocabulary;

pub use error::GraphError;
pub use search::LexicalHit;

/// A memory as projected, with its applied tags. Soft-deleted memories are never returned here.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct MemoryView {
    pub id: MemoryId,
    pub name: MemoryName,
    pub description: String,
    pub volatility: Volatility,
    pub created_at: Timestamp,
    pub tags: Vec<TagName>,
}

/// A live entry that carries a recurrence rule, with the memory it belongs to and the rule text —
/// the projection behind the console's per-memory recurring list, so the operator reads which rooms
/// carry recurring occurrences from the graph rather than a re-fold of the log (see
/// [`Graph::recurring_entries`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RecurringEntry {
    pub memory: MemoryId,
    pub text: String,
    pub rrule: String,
}

/// A content entry as projected, ordered within its memory by commit order. `occurred_sort` is the
/// denormalized representative instant of the entry's `occurred_at` (spec §Time), or `None` when the
/// entry carries no occurrence (or only a `Recurring` one); recency ranking reads it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct EntryView {
    pub entry_id: EntryId,
    pub asserted_at: Timestamp,
    pub occurred_sort: Option<Timestamp>,
    /// The entry's typed occurrence — when the fact happens — or `None` if undated. Carried alongside
    /// the flattened `occurred_sort` so a read can render the date faithfully (a recurrence or range,
    /// not just its sort instant), letting the agent see *when* on read instead of inspecting a
    /// structured field or searching for a date that lives outside the entry text.
    pub occurred_at: Option<TemporalRef>,
    /// Whether `occurred_at` was authored at append — the agent stamped it — rather than inferred
    /// later by the turn-end temporal extraction. Authored is ground truth; extracted is inference,
    /// so a representative-date projection prefers an authored occurrence over an extracted one, and a
    /// guessed date never shadows a stated one. `false` for an undated entry (it has no occurrence to
    /// classify) and for one whose occurrence was resolved by extraction.
    pub occurred_authored: bool,
    pub text: String,
    pub told_by: Teller,
    pub told_in: Option<ConversationRef>,
    pub visibility: Visibility,
    /// The entry that replaced this one, when it has been superseded (spec §Visibility → superseded
    /// entries are not live). `None` for a live entry. Live reads exclude superseded entries in SQL;
    /// this field surfaces on the history reads that deliberately include them. A retracted entry
    /// carries its *own* id here — the self-referential tombstone that makes every `superseded_by IS
    /// NULL` live filter hide it — so a consumer distinguishes a retraction from a supersession by
    /// `retracted_reason`, never by reading this as a successor.
    pub superseded_by: Option<EntryId>,
    /// The stated reason this entry was retracted, or `None` for a live or plainly-superseded entry.
    /// Present only on the history reads (a retraction drops from every live surface); the surfaces
    /// that show a retracted entry render this reason beside it.
    pub retracted_reason: Option<String>,
    /// Where this entry came from, derived from the recording event's [`EventSource`]. The one
    /// distinction the projection preserves is a connector-maintained entry (a participant's
    /// username, display name, or nickname projected and owned by a platform connector) versus an
    /// ordinary recorded one, since the maintenance cleanup passes must never mutate a
    /// connector-owned entry — the connector may supersede or retract it at any time.
    pub origin: EntryOrigin,
}

/// Where a content entry came from, projected from the recording event's [`EventSource`]. Kept
/// deliberately coarse: the only distinction a reader (and the cleanup passes) needs is whether the
/// entry is maintained by a platform connector or was recorded by the agent, operator, orchestration,
/// or genesis. A connector-owned entry is excluded from every autonomous cleanup pass, since the
/// connector holds its id and supersedes or retracts it as the platform-side account changes.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum EntryOrigin {
    /// Recorded by the agent, operator, orchestration, or genesis — everything that is not a platform
    /// connector. The default, and the origin of the overwhelming majority of entries.
    #[default]
    Recorded,
    /// Projected and maintained by a platform connector; the string names the platform it serves
    /// (mirroring [`EventSource::PlatformConnector`]). The cleanup passes never mutate such an entry.
    PlatformConnector(String),
}

impl EntryOrigin {
    /// The origin an entry recorded under `source` carries. Only a connector source is distinguished;
    /// every other source folds to [`EntryOrigin::Recorded`].
    pub fn from_source(source: &EventSource) -> EntryOrigin {
        match source {
            EventSource::PlatformConnector(platform) => {
                EntryOrigin::PlatformConnector(platform.clone())
            }
            _ => EntryOrigin::Recorded,
        }
    }

    /// Whether this entry is maintained by a platform connector — the case every cleanup pass excludes.
    pub fn is_connector(&self) -> bool {
        matches!(self, EntryOrigin::PlatformConnector(_))
    }
}

/// A registered relation as projected.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RelationView {
    pub name: RelationName,
    pub inverse: RelationName,
    pub from_card: Cardinality,
    pub to_card: Cardinality,
    pub symmetric: bool,
    pub reflexive: bool,
    /// The relation's one-line purpose, surfaced in the prompt and `links.list`/`get`.
    pub description: String,
}

/// A tag in the vocabulary as projected: its name, its one-line purpose, and how many live memories
/// carry it. Backs `tags.list` and the system prompt's tag-vocabulary block.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct TagVocabularyEntry {
    pub name: TagName,
    pub description: String,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub count: usize,
}

/// The fields the link visibility predicate reasons over, extracted from any link view type. Keeps
/// the predicate decoupled from the specific view shape (`LinkView`, `ClassLinkView`,
/// `NeighborLinkView`) each caller holds. The `told_in` field is carried so the marker can resolve
/// the reference, mirroring how content entries carry `told_in` for `MarkerTurn`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkVis {
    pub from: MemoryId,
    pub to: MemoryId,
    pub visibility: Visibility,
    pub told_by: Option<Teller>,
    pub told_in: Option<ConversationRef>,
}

/// A stored edge in its canonical direction, carrying its visibility posture and provenance.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct LinkView {
    pub from: MemoryId,
    pub to: MemoryId,
    pub relation: RelationName,
    /// The teller who asserted the relationship, if one is on record. `None` for links with no
    /// teller behind them (an operator-authored `same_as`) or predating link provenance.
    pub told_by: Option<Teller>,
    /// The conversation reference (turn or room) the link was asserted in, mirroring content
    /// entries' `told_in`.
    pub told_in: Option<ConversationRef>,
    /// The audience posture, governing the read-time `link_visible` predicate.
    pub visibility: Visibility,
}

impl LinkView {
    /// The fields the link visibility predicate reasons over.
    pub fn link_vis(&self) -> LinkVis {
        LinkVis {
            from: self.from,
            to: self.to,
            visibility: self.visibility.clone(),
            told_by: self.told_by.clone(),
            told_in: self.told_in.clone(),
        }
    }
}

/// A stored edge touching a `same_as` class, carrying its `source` so a class-traversing link read
/// keeps the per-edge provenance the agent-facing readers surface (spec §Lua API → link readers).
/// Distinct from [`LinkView`] so the console wire contract over the latter stays untouched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClassLinkView {
    pub from: MemoryId,
    pub to: MemoryId,
    pub relation: RelationName,
    pub source: LinkSource,
    /// The teller who asserted the relationship, if one is on record — `None` for a link with no
    /// teller behind it (an operator-authored `same_as`) or one predating link provenance.
    pub told_by: Option<Teller>,
    /// The conversation reference (turn or room) the link was asserted in, mirroring content
    /// entries' `told_in`.
    pub told_in: Option<ConversationRef>,
    /// The audience posture, governing the read-time `link_visible` predicate.
    pub visibility: Visibility,
}

impl ClassLinkView {
    /// The fields the link visibility predicate reasons over.
    pub fn link_vis(&self) -> LinkVis {
        LinkVis {
            from: self.from,
            to: self.to,
            visibility: self.visibility.clone(),
            told_by: self.told_by.clone(),
            told_in: self.told_in.clone(),
        }
    }
}

/// A neighbor on a memory's out-of-class relation surface — the raw material for the salient-relations
/// line a search hit carries. It names the relation, whether the edge runs *into* this class
/// (`incoming`) or out of it, and the far memory (id plus its resolved name, so a caller renders
/// `relation → name` without a second lookup). The query returns only edges leaving the class — an edge
/// internal to the `same_as` class is identity plumbing, not a relationship — ordered most-recently
/// created first (by the link's insertion `rowid`). Committed state; visibility-filtered through
/// `link_visible` when an audience is present, mirroring the content entry reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NeighborLinkView {
    pub relation: RelationName,
    pub incoming: bool,
    pub other: MemoryId,
    pub other_name: MemoryName,
    /// The `from` endpoint of the stored edge, pre-canonicalization. Needed so the predicate can
    /// reason about which endpoint is the teller and which is the subject.
    pub from: MemoryId,
    /// The `to` endpoint of the stored edge, pre-canonicalization.
    pub to: MemoryId,
    /// The teller who asserted the relationship, if one is on record.
    pub told_by: Option<Teller>,
    /// The conversation reference (turn or room) the link was asserted in, mirroring content
    /// entries' `told_in`.
    pub told_in: Option<ConversationRef>,
    /// The audience posture, governing the read-time `link_visible` predicate.
    pub visibility: Visibility,
}

impl NeighborLinkView {
    /// The fields the link visibility predicate reasons over.
    pub fn link_vis(&self) -> LinkVis {
        LinkVis {
            from: self.from,
            to: self.to,
            visibility: self.visibility.clone(),
            told_by: self.told_by.clone(),
            told_in: self.told_in.clone(),
        }
    }
}

/// A conversation as projected: its id, its locator (the room it addresses), and the context memory
/// that is its room. A conversation whose context memory has been deleted is not projected, so a
/// listing reflects only the live rooms (see [`Graph::conversations`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConversationView {
    pub id: ConversationId,
    pub locator: ConversationLocator,
    pub context_memory: MemoryId,
}

/// A session as projected: its conversation, when it opened, the carryover extent (if it opened via
/// compaction), the captured brief, and its participants (the present set at open, plus anyone who
/// joined mid-session).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionView {
    pub id: SessionId,
    pub conversation: ConversationId,
    pub started_at: Timestamp,
    pub seeded_from_turn: Option<ConversationRef>,
    pub brief: String,
    pub participants: Vec<MemoryId>,
}

/// What reconstructing a live `OpenSession` after a restart needs (see [`Graph::last_open_session`]):
/// the session's id, the brief frozen at its open, when it opened, and the `SessionStarted` seq the
/// live buffer reads from. `seeded` flags a compaction-seam continuation, whose true buffer starts at
/// a carried tail before `start_seq` — so it is not byte-faithfully resumable from the seq alone.
#[derive(Clone, Debug, PartialEq)]
pub struct OpenSessionView {
    pub id: SessionId,
    pub brief: String,
    pub started_at: Timestamp,
    pub start_seq: Seq,
    pub seeded: bool,
}

/// The plan for minting a fresh [`Namespace::Person`] participant stub: the qualified name it
/// receives (`person/<id>@<platform>`). The caller (`resolve_or_mint_participant`) is responsible
/// for checking whether the name already exists as a memory (an agent-authored hearsay stub) and
/// binding the platform identity to it, or creating a fresh memory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParticipantMint {
    pub name: MemoryName,
}

/// A failure projecting or querying the graph.
pub struct Graph {
    conn: Connection,
}

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
    fn guard_schema(&self) -> Result<(), GraphError> {
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

    /// The highest `Seq` applied so far, or `Seq::ZERO` for a fresh graph. Replay resumes from
    /// `head().next()`, which is how a stale graph catches up to log-head.
    pub fn head(&self) -> Result<Seq, GraphError> {
        let value: Option<i64> = self
            .conn
            .query_row("SELECT value FROM meta WHERE key = 'graph_head'", [], |r| {
                r.get(0)
            })
            .optional()
            .map_err(backend)?;
        Ok(Seq(value.unwrap_or(0) as u64))
    }

    /// Replay every event the store holds beyond the current head, applying each. Returns the count
    /// applied. The same machinery catches up a stale graph and rebuilds a fresh one.
    pub fn materialize_from(&mut self, store: &dyn Store) -> Result<usize, GraphError> {
        let from = self.head()?.next();
        let events = store.read_from(from).map_err(GraphError::Store)?;
        for event in &events {
            self.apply(event)?;
        }
        tracing::debug!(applied = events.len(), from = from.0, "materialized graph");
        Ok(events.len())
    }

    /// Write an atomic checkpoint of the graph to `path` via `VACUUM INTO` (spec §Snapshots). The
    /// `meta.graph_head` rides along in the copy, so the file is a self-describing graph at the head it
    /// was captured at. `path` must not already exist (SQLite refuses to overwrite). The caller is
    /// responsible for capturing at a clean `seq` boundary — no in-flight commit — which holding the
    /// graph lock across this call guarantees (commits take the same lock).
    pub fn snapshot_into(&self, path: &std::path::Path) -> Result<(), GraphError> {
        self.conn
            .execute("VACUUM INTO ?1", params![path.to_string_lossy()])
            .map_err(backend)?;
        Ok(())
    }

    /// A content fingerprint over the graph's logical state — every row of every projected table, in a
    /// canonical order independent of physical layout — so two graphs can be compared for equality: a
    /// snapshot against its source, or a graph rebuilt by replay against the original. Stable across
    /// `VACUUM` (which rebuilds the physical layout but preserves logical content) because it reads only
    /// declared columns in a content order, never implicit rowids. Excludes the derived FTS index,
    /// which is a function of the base tables. `meta` is included, so two graphs match only if they are
    /// at the same `graph_head`.
    pub fn fingerprint(&self) -> Result<String, GraphError> {
        // Every projected table, in a fixed order. The FTS index (`memories_fts`) is derived from
        // these, so it is left out rather than hashing its virtual-table internals.
        const TABLES: &[&str] = &[
            "memories",
            "content_entries",
            "tags",
            "memory_tags",
            "relations",
            "links",
            "conversations",
            "sessions",
            "session_participants",
            "participant_identities",
            "meta",
        ];
        let mut hasher = Sha256::new();
        for table in TABLES {
            hasher.update(table.as_bytes());
            hasher.update([SEP_TABLE]);
            // Order by every column (by position), so the row sequence is a function of content, not of
            // physical layout — the property that makes the digest VACUUM-stable.
            let column_count = self
                .conn
                .prepare(&format!("SELECT * FROM {table}"))
                .map_err(backend)?
                .column_count();
            let order = (1..=column_count)
                .map(|index| index.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            let mut stmt = self
                .conn
                .prepare(&format!("SELECT * FROM {table} ORDER BY {order}"))
                .map_err(backend)?;
            let mut rows = stmt.query([]).map_err(backend)?;
            while let Some(row) = rows.next().map_err(backend)? {
                for index in 0..column_count {
                    hash_value(&mut hasher, row.get_ref(index).map_err(backend)?);
                }
                hasher.update([SEP_ROW]);
            }
        }
        Ok(hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect())
    }
}

/// The raw memory columns the `memories` SELECT yields; consumed by [`Graph::assemble_memory`].
pub(super) type MemoryColumns = (String, String, String, String, i64);

/// Shared memory-decoding and tag reads used across the sub-domain query modules.
impl Graph {
    /// Assemble a [`MemoryView`] from its raw column tuple, decoding the id and volatility and loading
    /// the memory's tags. Shared by every memory query that selects the standard `memories` columns.
    fn assemble_memory(&self, columns: MemoryColumns) -> Result<MemoryView, GraphError> {
        let (id, name, description, volatility, created_at) = columns;
        Ok(MemoryView {
            id: MemoryId(parse_ulid(&id)?),
            name: MemoryName::new(name),
            description,
            volatility: volatility.parse().map_err(|()| {
                GraphError::Malformed(format!("unknown volatility {volatility:?}"))
            })?,
            created_at: timestamp_column(created_at, "created_at")?,
            tags: self.tags_of(&id)?,
        })
    }

    fn tags_of(&self, memory_id: &str) -> Result<Vec<TagName>, GraphError> {
        let stmt = self
            .conn
            .prepare("SELECT tag FROM memory_tags WHERE memory_id = ?1 ORDER BY tag")?;
        query_map_into(stmt, params![memory_id], |row| {
            let tag: String = row.get(0)?;
            Ok(TagName::new(&tag))
        })
    }
}

/// Row and table separators in the fingerprint stream, so distinct row/table boundaries cannot be
/// forged by content (e.g. two short fields colliding with one longer field).
const SEP_ROW: u8 = 0xFF;
const SEP_TABLE: u8 = 0xFE;

/// Feed one SQLite value into the fingerprint hasher, tagged by type and length-prefixed, so values of
/// different types or lengths can never produce the same byte stream.
fn hash_value(hasher: &mut Sha256, value: ValueRef<'_>) {
    match value {
        ValueRef::Null => hasher.update([0]),
        ValueRef::Integer(int) => {
            hasher.update([1]);
            hasher.update(int.to_le_bytes());
        }
        ValueRef::Real(real) => {
            hasher.update([2]);
            hasher.update(real.to_le_bytes());
        }
        ValueRef::Text(text) => {
            hasher.update([3]);
            hasher.update((text.len() as u64).to_le_bytes());
            hasher.update(text);
        }
        ValueRef::Blob(blob) => {
            hasher.update([4]);
            hasher.update((blob.len() as u64).to_le_bytes());
            hasher.update(blob);
        }
    }
}

fn parse_ulid(text: &str) -> Result<Ulid, GraphError> {
    Ulid::from_string(text)
        .map_err(|e| GraphError::Malformed(format!("invalid ulid {text:?}: {e}")))
}

/// Decode a `column`'s epoch-millisecond value into a [`Timestamp`], or a [`GraphError::Malformed`]
/// if it falls outside jiff's representable range — the projection is derived from the event log, so
/// an out-of-range value here means the log itself carries a value the wire's [`Timestamp`] serde
/// impl would have rejected, not a value this read can silently coerce.
pub(super) fn timestamp_column(millis: i64, column: &str) -> Result<Timestamp, GraphError> {
    Timestamp::try_from_millis(millis).ok_or_else(|| {
        GraphError::Malformed(format!(
            "{column} {millis} milliseconds since the Unix epoch is outside the representable range"
        ))
    })
}

/// The stamp `guard_schema` compares: the leading eight bytes of a SHA-256 over the schema DDL,
/// stored in `meta` as an integer. A digest of the DDL text itself, so the stamp is a pure function
/// of the schema with no versioning discipline to uphold.
fn schema_fingerprint() -> i64 {
    let digest = Sha256::digest(Graph::SCHEMA_SQL.as_bytes());
    i64::from_le_bytes(
        digest[..8]
            .try_into()
            .expect("a SHA-256 digest holds at least eight bytes"),
    )
}

pub(super) fn backend(error: rusqlite::Error) -> GraphError {
    GraphError::Backend(error)
}
