# Maintenance passes

Maintenance passes are autonomous data-hygiene machinery that runs off the hot path. They run on a timer, gated on activity — a pass that finds nothing to do is cheap, and a pass that runs too soon after the last one wastes a model call. They can also be invoked on demand via the CLI or the control API.

## Scheduling

Each maintenance pass gates on "how much has changed since the last run" rather than ticking blindly. A pass tracks events since its last cursor advance, and below a threshold the tick is a no-op without even a graph read. The threshold is a per-pass settings knob (`consolidation_min_activity`, `canonicalize_min_activity`, `link_cleanup_min_activity`).

The maintenance driver ticks every `tick_seconds` (default 60). Each tick checks each pass's activity gate and runs the pass if the gate fires. The passes are cursor-resumed and idempotent, so an idle tick is cheap.

## The passes

### Consolidation

Clusters semantically-overlapping live entries within each identity class and synthesizes a single richer consolidated entry that preserves their interrelated clauses. The source entries are tombstoned (stamped `superseded_by` = the replacement entry id), dropping them from live surfaces while preserving them in history. A new `EntriesConsolidated` event carries the full many-to-one relationship.

The pass also absorbs entries whose content is purely a description of a link that exists — the consolidation model decides whether an entry's content is fully captured by existing links and drops it during synthesis.

Consolidated entries carry `Teller::Agent` and the least-restrictive visibility of their sources. Same-visibility clusters consolidate to that class; cross-visibility near-duplicates consolidate to the least restrictive (public > attributed > private).

#### Two embedding spaces

Each entry has two embedding vectors, maintained in lockstep by the indexer:

- **`Entry`** — the raw entry text. Serves search, where the query has no subject-name prefix.
- **`EntryContextual`** — `"{handle}: {text}"`. Serves the dedup check and consolidation pass, where entries within the same memory are compared. The handle prefix normalizes entries that include the subject name with those that don't — without it, "Rowan is a senior developer" and "is a senior developer" score ~0.52 cosine despite being the same fact, because the name token dominates the embedding.

Both spaces are GC'd on supersession, retraction, and consolidation. The `Entry` space is unaffected by renames; the `EntryContextual` space becomes stale after a rename (the prefix changes) until the entry is next re-embedded — an accepted floor, since the stale embedding still works (it just has the old prefix).

After upgrading an existing agent, the `EntryContextual` space starts empty. Run `zuihitsu debug reindex` (followed by a restart) to rebuild the full vector index from the log. The indexer's normal catch-up handles new entries; old entries get their contextual vectors when they're next re-embedded (on content change or consolidation).

### Canonical profiles

Gives platform stubs (`person/<id>@<platform>`) readable named identities. The pass reads a stub's entries, calls the model to identify the most name-like text, and mints a bare `person/<name>` canonical profile. If the name already exists for a different person, a disambiguated profile (`person/<name>-2`, etc.) is created.

The canonical profile is bound to the stub via a `same_as` link (asserted under `Authority::Agent`, which permits direct assertion without operator confirmation) and designated as the class primary. This is the "free merge" case — the canonical profile is empty, so there is no visibility risk.

### Link-redundant entry cleanup

Retracts entries whose content is purely a description of a link that exists. For example, an entry "knows Dave" that spawned a `knows → person/dave` link is redundant once the link exists. The pass runs after consolidation, so it sees the consolidated entry set. An entry that carries detail beyond the link (e.g. "met Dave at the climbing gym last Tuesday") is preserved.

## Authority::Agent

Maintenance passes run under a new `Authority::Agent` authority tier, which is narrower than full self-evolution:

- **Permits cross-teller supersede**: the consolidation pass can supersede entries told by different participants (the foreign-confidence gate passes for non-Platform authority).
- **Permits free `same_as` assertion**: the canonical-profile pass asserts `same_as` directly without routing to a merge proposal (the `same_as`-routes-to-proposal gate fires only under Platform authority).
- **Blocks `self` writes**: `guard_self` blocks all non-Operator authority, so neither maintenance pass can touch the self model.

## Settings

```toml
[maintenance]
enabled = true                    # whether passes fire on the timer
tick_seconds = 60                 # how often the driver ticks
consolidation_min_activity = 20   # min events since last consolidation run
canonicalize_min_activity = 5     # min events since last canonicalize run
link_cleanup_min_activity = 20    # min events since last link-cleanup run
consolidation_similarity_threshold = 0.85  # cosine threshold for clustering
dedup_similarity_threshold = 0.95          # cosine threshold for append-time dedup
```

## CLI invocation

```
zuihitsu maintenance consolidate     # run the consolidation pass
zuihitsu maintenance canonicalize     # run the canonical-profile pass
zuihitsu maintenance link-cleanup     # run the link-redundant entry cleanup
```

## Control API

- `POST /control/maintenance/consolidate` — drive the consolidation pass.
- `POST /control/maintenance/canonicalize` — drive the canonical-profile pass.
- `POST /control/maintenance/link-cleanup` — drive the link-redundant entry cleanup.

## Relationship to existing background passes

The existing background passes — describe (description regeneration) and link inference — are also cursor-resumed and run on timers. The maintenance passes share the same `BackgroundPasses` infrastructure (cursors, guards, catch-up methods) but are heavier (they call the model for synthesis) and so tick at a longer interval by default.
