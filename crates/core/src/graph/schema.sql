-- The materialised graph's projection schema: one idempotent DDL batch, applied at every open and
-- re-applied wholesale after a schema-fingerprint mismatch resets the graph (see schema.rs — the
-- fingerprint is a digest of this file, so any edit here moves the stamp). Derived store: every
-- table rebuilds from the event log, so changes need no migration, only this file.
--
-- Every ordinary table is STRICT, and the references the write path maintains are declared as
-- FOREIGN KEY clauses enforced from the first write. No FK exists for
-- `relations.inverse`/`from_card`/`to_card` (string enums) or `sessions.seeded_from_turn` (a turn
-- is not a row); every "no parent yet" slot is already NULL-able, so no auxiliary boolean column
-- is needed.

CREATE TABLE IF NOT EXISTS memories (
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
    last_described_seq INTEGER NOT NULL DEFAULT 0,
    -- A memory is born its own class (`class_id` = its own id); recompute_classes later re-points
    -- it at a class anchor, itself a `memories.id`.
    FOREIGN KEY (class_id) REFERENCES memories(id)
) STRICT;

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
    seq           INTEGER NOT NULL,
    -- Entries persist for replay, audit, and `BeforeAfter` anchor resolution after their memory is
    -- soft-deleted, and are never hard-deleted, so both FKs are plain.
    FOREIGN KEY (memory_id) REFERENCES memories(id),
    FOREIGN KEY (superseded_by) REFERENCES content_entries(entry_id)
) STRICT;

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
    PRIMARY KEY (entry_id, teller),
    -- A consolidation carries attestations from a retired entry (`source_entry`), so both FKs are
    -- plain over immutable entry rows.
    FOREIGN KEY (entry_id) REFERENCES content_entries(entry_id),
    FOREIGN KEY (source_entry) REFERENCES content_entries(entry_id)
) STRICT;

CREATE INDEX IF NOT EXISTS idx_entry_attestations_entry
    ON entry_attestations(entry_id, seq);

CREATE TABLE IF NOT EXISTS entry_disputes (
    entry_id  TEXT PRIMARY KEY,
    memory_id TEXT NOT NULL,
    statement TEXT NOT NULL,
    FOREIGN KEY (memory_id) REFERENCES memories(id),
    -- The dispute row is re-inserted per arbitration cycle, so the FK is plain.
    FOREIGN KEY (entry_id) REFERENCES content_entries(entry_id)
) STRICT;

CREATE INDEX IF NOT EXISTS idx_entry_disputes_memory
    ON entry_disputes(memory_id);

CREATE TABLE IF NOT EXISTS tags (
    name        TEXT PRIMARY KEY,
    description TEXT NOT NULL
) STRICT;

CREATE TABLE IF NOT EXISTS memory_tags (
    memory_id TEXT NOT NULL,
    tag       TEXT NOT NULL,
    PRIMARY KEY (memory_id, tag),
    -- A deleted memory's tag rows die with it, so the memory FK cascades. Tags are never deleted,
    -- so the tag FK is plain. Inserted via `INSERT OR IGNORE`, which swallows FK violations like
    -- any constraint: a duplicate application is suppressed, and an application to an absent
    -- parent silently drops the row.
    FOREIGN KEY (memory_id) REFERENCES memories(id) ON DELETE CASCADE,
    FOREIGN KEY (tag) REFERENCES tags(name)
) STRICT;

CREATE TABLE IF NOT EXISTS relations (
    name        TEXT    PRIMARY KEY,
    inverse     TEXT    NOT NULL,
    from_card   TEXT    NOT NULL,
    to_card     TEXT    NOT NULL,
    symmetric   INTEGER NOT NULL,
    reflexive   INTEGER NOT NULL,
    description TEXT    NOT NULL DEFAULT ''
) STRICT;

CREATE TABLE IF NOT EXISTS links (
    from_id     TEXT    NOT NULL,
    to_id       TEXT    NOT NULL,
    relation    TEXT    NOT NULL,
    source      TEXT    NOT NULL,
    told_by     TEXT,
    told_in     TEXT,
    visibility  TEXT    NOT NULL DEFAULT 'Public',
    asserted_at INTEGER NOT NULL,
    PRIMARY KEY (from_id, to_id, relation),
    -- A soft-deleted memory keeps its link rows (deletion filters reads, never the tables), so both
    -- endpoint FKs are plain. The relation FK is plain too: relations are never deleted, and an
    -- unregistered label fails the FK rather than dangling.
    FOREIGN KEY (from_id) REFERENCES memories(id),
    FOREIGN KEY (to_id) REFERENCES memories(id),
    FOREIGN KEY (relation) REFERENCES relations(name)
) STRICT;

CREATE INDEX IF NOT EXISTS idx_links_to ON links(to_id, relation);

CREATE TABLE IF NOT EXISTS memory_aliases (
    former_name TEXT PRIMARY KEY,
    memory_id   TEXT NOT NULL,
    -- A renamed memory's former-name alias persists, so the FK is plain.
    FOREIGN KEY (memory_id) REFERENCES memories(id)
) STRICT;

CREATE TABLE IF NOT EXISTS conversations (
    id             TEXT    PRIMARY KEY,
    platform       TEXT    NOT NULL,
    scope_path     TEXT    NOT NULL,
    context_memory TEXT    NOT NULL,
    ended          INTEGER NOT NULL DEFAULT 0,
    -- Deleting a context memory drops its conversation (the room is the conversation's identity;
    -- see the `MemoryDeleted` cascade), so the FK cascades.
    FOREIGN KEY (context_memory) REFERENCES memories(id) ON DELETE CASCADE
) STRICT;

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
    seq              INTEGER NOT NULL,
    -- A deleted context memory's cascade takes the conversation's sessions with it.
    FOREIGN KEY (conversation) REFERENCES conversations(id) ON DELETE CASCADE
) STRICT;

CREATE INDEX IF NOT EXISTS idx_sessions_conversation ON sessions(conversation);

CREATE TABLE IF NOT EXISTS session_participants (
    session TEXT NOT NULL,
    memory  TEXT NOT NULL,
    at_turn TEXT,
    PRIMARY KEY (session, memory),
    -- The `MemoryDeleted` cascade drops a conversation's sessions and their participants outright,
    -- so both FKs cascade.
    FOREIGN KEY (session) REFERENCES sessions(id) ON DELETE CASCADE,
    FOREIGN KEY (memory) REFERENCES memories(id) ON DELETE CASCADE
) STRICT;

CREATE TABLE IF NOT EXISTS participant_identities (
    platform         TEXT NOT NULL,
    platform_user_id TEXT NOT NULL,
    memory           TEXT NOT NULL,
    PRIMARY KEY (platform, platform_user_id),
    FOREIGN KEY (memory) REFERENCES memories(id) ON DELETE CASCADE
) STRICT;

CREATE INDEX IF NOT EXISTS idx_participant_identities_memory
    ON participant_identities(memory);

CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value INTEGER NOT NULL) STRICT;

CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(
    name, description, content, memory_id UNINDEXED
);
