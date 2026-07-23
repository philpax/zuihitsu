# Maintenance passes

Maintenance passes are autonomous data-hygiene machinery that runs off the hot path. They run on a timer, gated on activity — a pass that finds nothing to do is cheap, and a pass that runs too soon after the last one wastes a model call. They can also be invoked on demand via the CLI or the control API.

## Scheduling

Each maintenance pass gates on "how much has changed since the last run" rather than ticking blindly. A pass tracks events since its last cursor advance, and below a threshold the tick is a no-op without even a graph read. The threshold is a per-pass settings knob (`consolidation_min_activity`, `canonicalize_min_activity`, `link_cleanup_min_activity`).

The maintenance driver ticks every `tick_seconds` (default 60). Each tick checks each pass's activity gate and runs the pass if the gate fires. The passes are cursor-resumed and idempotent, so an idle tick is cheap.

## Born-instance caveat

Each pass drives a prompt template — consolidation the `EntryConsolidation` template, canonicalize the `NameIdentification` template, and link cleanup the `LinkCleanup` template — and returns early as a silent no-op (advancing its cursor) when that template is absent from the log. An agent born before these template names existed lacks them at genesis, but acquires them without operator action: `genesis::reconcile_new_templates` runs at every boot and additively registers any build-default template whose name has never appeared on the log (see [Prompt templates](lifecycle.md#prompt-templates)). So a born agent picks up a newly-shipped template name at its next boot, and the maintenance passes become active from then on. A pass only no-ops when the running binary itself predates the template — a boot on an old binary from before the reconcile logic existed.

## The passes

### Consolidation

Consolidation runs in two tiers per identity class, each committed as its own block through the ordinary `MemoryBlock` write path under `Authority::Agent`.

**Tier 1 — within-level synthesis.** Live entries are grouped by visibility posture before clustering: `Public` entries merge across tellers (at the public level the teller is provenance, not the audience-bearing payload), while `Attributed`, `PrivateToTeller`, and `Exclude` entries group per teller — and, for `Exclude`, per exact withheld set — since below the public level the teller determines who may see the fact. Within a group, near-duplicates are clustered at `consolidation_similarity_threshold` and the model synthesizes a single richer entry that preserves their interrelated clauses. The replacement inherits the group's visibility verbatim, and its teller is the group's teller when uniform or `Teller::Agent` for a cross-teller public merge. Because synthesis never crosses a level, a private confidence's text is never folded into a copy visible to a wider audience.

**Tier 2 — cross-level dedup, never synthesis.** After tier 1 commits, a more-private live entry whose fact is already attested by a more-public one — measured at the stricter `dedup_similarity_threshold`, not the looser consolidation bar — is retired into that public entry. No new text is written: the `EntriesConsolidated` names the existing public entry as the replacement, and the private text enters no prompt. A fact already attested at least as widely is redundant in its private copy, and the stricter threshold is where "same fact" is credible enough to act on. Only a genuinely private source (`PrivateToTeller` or `Exclude`) is eligible, and only an all-audience entry (`Public` or `Attributed`) is a valid replacement, so the replacement's audience is always a superset of the retired entry's — never an intersected or rotated one.

In both tiers the source entries are tombstoned (stamped `superseded_by` = the replacement entry id), dropping them from live surfaces while preserving them in history, and each `EntriesConsolidated` event carries the full many-to-one relationship.

A connector-maintained entry is excluded from consolidation entirely. Each content entry carries an `EntryOrigin` derived from its recording event's source: an entry recorded by a platform connector (`EntryOrigin::PlatformConnector`) is never grouped, so it can be neither a source nor a replacement in either tier. The connector holds that entry's id and supersedes or retracts it as the platform-side account changes; folding it into a synthesized replacement, or retiring another entry into it, would strand that maintenance.

#### Two embedding spaces

Each entry has two embedding vectors, maintained in lockstep by the indexer:

- **`Entry`** — the raw entry text. Serves search, where the query has no subject-name prefix.
- **`EntryContextual`** — `"{handle}: {text}"`. Serves the dedup check and consolidation pass, where entries within the same memory are compared. The handle prefix normalizes entries that include the subject name with those that don't — without it, "Rowan is a senior developer" and "is a senior developer" score ~0.52 cosine despite being the same fact, because the name token dominates the embedding.

The split is deliberate: the two spaces serve opposite needs, so neither can serve both. The handle prefix that normalises entries for dedup measurably degrades search ranking — a query carries no subject-name prefix, so prefixing the indexed text pulls it away from the query — which is why the raw `Entry` space serves search while the `EntryContextual` space serves dedup and consolidation.

Both spaces are GC'd on supersession, retraction, and consolidation. The `Entry` space is unaffected by renames; the `EntryContextual` space becomes stale after a rename (the prefix changes) until the entry is next re-embedded — an accepted floor, since the stale embedding still works (it just has the old prefix).

After upgrading an existing agent, the `EntryContextual` space starts empty. Run `zuihitsu debug reindex` (followed by a restart) to rebuild the full vector index from the log. The indexer's normal catch-up handles new entries; old entries get their contextual vectors when they're next re-embedded (on content change or consolidation).

`zuihitsu debug embed <a> <b>` is the distinct similarity-tuning tool: it embeds two strings through the configured endpoint and prints their cosine similarity, so the dedup and consolidation thresholds can be re-validated against real phrasings when the embedding model or the thresholds change.

### Canonical profiles

Gives platform stubs (`person/<id>@<platform>`) readable named identities. The pass reads a stub's entries, calls the model to identify the most name-like text, and mints a bare `person/<name>` canonical profile. If the name already exists for a different person, a disambiguated profile (`person/<name>-2`, etc.) is created.

The canonical profile is bound to the stub via a `same_as` link (asserted under `Authority::Agent`, which permits direct assertion without operator confirmation) and designated as the class primary. This is the "free merge" case — the canonical profile is empty, so there is no visibility risk.

### Link-redundant entry cleanup

Retracts entries whose content is purely a description of a link that exists. For example, an entry "knows Dave" that spawned a `knows → person/dave` link is redundant once the link exists. The pass runs after consolidation, so it sees the consolidated entry set. An entry that carries detail beyond the link (e.g. "met Dave at the climbing gym last Tuesday") is preserved.

A connector-maintained entry (`EntryOrigin::PlatformConnector`) is dropped from the candidate set, so this pass never retracts a connector-owned entry: the connector holds its id and supersedes or retracts it as the platform-side account changes.

## Authority::Agent

Maintenance passes run under a new `Authority::Agent` authority tier, which is narrower than full self-evolution. All three passes drive their writes through the ordinary `MemoryBlock` write path under this authority — each buffering its events in a block and committing them under `EventSource::Orchestration` — so every consolidation, mint, `same_as`, designation, and retraction clears the same guards a turn's writes do rather than bypassing them as raw appends. The tier's distinguishing powers:

- **Clears the foreign-confidence supersede guard**: the guard blocks a platform turn from retiring another participant's confidence, but tier-2 dedup is the deliberate exception — it retires a private copy only when the same fact is already attested by an all-audience entry at the stricter dedup threshold, so nothing is suppressed that was not already visible at least as widely. `Authority::Agent` clears the guard for this case; the pass's superset-audience check is what makes clearing it sound.
- **Permits free `same_as` assertion**: the canonical-profile pass asserts `same_as` directly without routing to a merge proposal (the `same_as`-routes-to-proposal gate fires only under Platform authority).
- **Blocks `self` writes**: `guard_self` blocks all non-Operator authority, so no maintenance pass can touch the self model.

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
