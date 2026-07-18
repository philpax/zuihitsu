# Initialisation and lifecycle

Initialisation is just the first events in the log; there is no separate config-state. There are two kinds of "config" — operational, which stays out of the log, and behavioural, which is event-sourced — plus the genesis events that seed the log:

- **Operational (environmental) config** (a TOML file, `EnvConfig` in `src/config/mod.rs`): where and how the instance runs — storage location, model and embedding endpoints, serving, and MCP servers; the field breakdown is under [Configuration](#configuration). It is environmental, not behavioural: it changes when you move machines, not when the agent learns something, and it stays out of the log. There is no "operator identity" here: the operator is whoever holds the console (a loopback peer, or a remote peer bearing a control key), not a configured platform principal (see [Trust model](trust-and-authority.md#trust-model)).
- **Behavioural config** (event-sourced via `ConfigSet`, seeded at genesis): the tunables that shape what the agent did and saw, so replay must know the value in force at the time. See [Configuration](#configuration) below for the breakdown.
- **Genesis events** (first entries in the log): prompt templates, seed link relations, seed tags, the default `ConfigSet`, and a minimal `self`. The smallest set of facts that must exist for the agent to function.

## Configuration

The dividing test is not "faithful replay needs the value." It doesn't, for almost any of these, because the outcome each value produced is already a logged fact: a boundary is a [`SessionStarted`](events-and-storage.md#event-sourcing), the brief is captured, the `max_steps` outcome is on the turn, and a search's result is in [`LuaExecuted`](events-and-storage.md#event-sourcing)`.result`. The real test is whether this is a tunable that shaped behaviour, such that you'd want to explain, vary, or detect drift in it.

If yes, it is behavioural and lives in the log, for three reasons: auditability (explaining why a boundary fell where it did, which the outcome alone doesn't reveal), counterfactual replay (re-running a sequence under varied weights to see how behaviour changes), and independence from the build (the settings snapshot is pinned in the agent's own log, so a later build shipping a new default cannot silently change how an existing agent behaves). If it only describes where and how the instance runs, it is environmental and lives in the file. The log contains everything needed to explain and re-examine why the agent did what it did; the file contains everything needed to run the instance.

The lone faithful-replay dependency — the carryover tail extent across a [compaction seam](conversations-and-briefs.md#compaction-token-triggered-re-segmentation) — is closed by recording it as a fact, `seeded_from_turn` on `SessionStarted`, rather than by consulting config; after that, no behavioural config is needed for faithful replay at all.

The behavioural settings are one strongly-typed struct (`Settings` in `crates/core/src/settings.rs`), grouped into per-subsystem substructs — compaction, checkpoint, brief, turn, search, scheduler, concurrency, observability, and memory — and carried whole in each `ConfigSet` event. The current settings are the latest snapshot (`Settings::from_store`). It is deliberately not a per-context policy language: per-context variation is better handled by the agent reasoning over `context/*` memory than by a config policy language.

The schema is **append-only**: a field is deprecated, never removed, and every substruct is `#[serde(default)]` over its `Default`, so every snapshot ever logged still loads, and a field absent from an older snapshot deserialises to its build default.

**Behavioural (event-sourced, `ConfigSet`) — illustrative fields:**

- *Compaction token budget* — when the buffer triggers a re-segment (determined where session boundaries fell). Derived from the model's context window at genesis when one is configured (`compaction_budget_for`), else the built-in default.
- *Idle-gap threshold* — the quiet period that ends a session (same: segmentation).
- *Carryover character budget* — how much raw transcript crosses a compaction boundary (what the agent saw next).
- *Flush gating threshold* (`flush_min_turns`) — whether a session was substantive enough to flush.
- *Checkpoint flush settings* — whether mid-session checkpoint flush is enabled, and its delta and cooldown gates.
- *Brief token budget* and *`recent_facts` count* — what entered each brief.
- *Present-set cap* — how many participants got full briefs.
- *`max_steps`* — whether a turn terminated normally or hit the bound (a recorded outcome); plus the per-block timeout and attempt bound.
- *Search scoring weights and recency-decay constants* — which memories retrieval surfaced.
- *Concurrent-stream limit* (`concurrency.max_concurrent_streams`) — how many turns may crowd the shared model at once. Logged rather than environmental: it is read from the snapshot at construction and bounds the turn semaphore.
- *Scheduler tick and per-session wake-up cap*, the *model-call capture level*, and the *maximum entry length*.

**Environmental (operational file, `EnvConfig`):**

- `[storage]` — a single `dir` holding all three databases (`events.sqlite`, `graph.sqlite`, `vectors.sqlite`), resolved relative to the config file's own directory. The event log is the source of truth; the graph and vector index are rebuildable projections.
- `[model]` — the generation endpoint and model id; the model's context window (`context_length`, required whenever an endpoint is set — the OpenAI-style API does not report it, and the compaction budget derives from it); the sampling parameters (`temperature`, `top_p`, `top_k`, `min_p`, `presence_penalty`, and a `thinking` override), each optional and simply not sent when unset, so the serving layer applies its own per-model default; and a `[model.resilience]` block (request timeout, bounded retries with exponential backoff, and a circuit breaker). Resilience is environmental precisely because retries the agent never saw emit nothing to the log, so replay never depends on them.
- `[embedding]` — the embedding endpoint, model id, dimensionality, request timeout, and optionally the model's context window in tokens (`context_length`), which bounds each input by truncation before it is embedded. The embedding model *identity* is environmental, but a *change* of it is the logged `EmbeddingModelChanged` migration: if the embedding model changed, boot clears and rebuilds the vector index before serving (see [Vector store](events-and-storage.md#vector-store)).
- `[serving]` — the HTTP bind address and the `control_keys` array for `/control/*`. A loopback peer is trusted without a key; a remote peer must present one of these keys as `Authorization: Bearer <key>`. An empty array (the default) rejects every remote control request, so binding a routable address is fail-closed safe (see [Trust model](trust-and-authority.md#trust-model)).
- `[connectors]` — the map of registered connectors, one entry per connector (`discord = { key = "…" }`). The map key is the connector's id, which is both its platform id and the id its writes are attributed to; the value carries its bearer key. Every `/platform/*` request is scoped to exactly one connector, resolved from its key, and the platform it acts on is that connector's id — never a value in the request body. A loopback peer needs no key and is confined to the reserved `direct` platform; a remote peer with no key, or an unrecognised one, is rejected. An empty map (the default) rejects every keyed platform request, leaving only loopback's `direct` (see [Trust model](trust-and-authority.md#trust-model)).
- `[snapshots]` — graph-snapshot cadence: whether snapshotting is enabled (on by default), where snapshots go, the check interval, the activity gate (`min_new_events`), and how many to retain. This affects replay speed, not replay result.
- `[mcp.<name>]` — one block per configured MCP server (schema below).

The keys the config carries never cross the wire back out: `control_keys` serialises as its *count*, each connector's key serialises away (leaving only the connector ids), and an MCP server's `env` serialises as its variable *names* only, so the config view (`GET /control/config`) cannot leak a secret.

## The environmental config is a TOML file, resolved per invocation

The config path is the `--config <path>` argument, defaulting to `config.toml` in the current working directory (a global argument, so it applies to the server and every operator subcommand alike). A missing file is not an error and is not written: it yields the in-memory `EnvConfig::default()`, so a bare instance still has somewhere — a `data/` directory beside the intended config — to put its databases. Relative storage and snapshot paths resolve against the config file's own directory, so an instance is relocatable by moving its directory.

Because this file carries the storage directory, that directory is the instance selector: the executable is stateless, and two configs with different storage directories and endpoints are two independent agents, each with its own event log, hence its own behavioural config (`ConfigSet`) and its own whole identity. That is how one executable runs several agents at once. The file says where this instance runs, not who the operator is: operator identity remains "a loopback peer, or a remote peer bearing a control key" (see [Trust model](trust-and-authority.md#trust-model)), never a credential in the file.

The defaults the code applies for an absent or partial file are fail-closed by construction: the bind defaults to loopback (`127.0.0.1:7777`) with empty key arrays, so a default instance is reachable only from its own host. The one safety gap the defaults do *not* close is path collision — the default storage directory is always `data/`, so two default-generated instances in the same working directory would point at the same log. That collision is caught at boot rather than in the defaults: the first writer holds an exclusive lock on the event log, and the second fails fast (see [Boot](#boot-every-startup) below) rather than corrupting it.

## MCP server blocks

Each configured MCP server (see [Lua API → External I/O via MCP](agent-loop.md#external-io-via-mcp)) is one table (`McpServerConfig` in `src/mcp/mod.rs`):

```toml
[mcp.browser]
command = "browser-mcp"                 # executable; argv, never shell-split
args    = ["mcp"]
env     = { FOO = "bar" }               # optional extra environment (serialised redacted)
cwd     = "/path"                       # optional working directory
allow   = ["navigate", "markdown", "links"]   # optional; raw tool names
deny    = ["evaluate"]                         # optional; raw tool names
```

The table key (`browser`) is the projection prefix `mcp.<key>.*`, so it MUST be a valid Lua identifier (`[A-Za-z_][A-Za-z0-9_]*`), rejected at config load otherwise (`ConfigError::InvalidMcpServerName`). `command` + `args` are an argv pair with no shell-splitting; zuihitsu rejects shell-splitting to avoid the shell-quoting footgun. Stdio is the only transport, so it is not a field. `allow` / `deny` are matched raw against the server's advertised catalogue during the one-time startup probe (`McpCatalogue::probe`): `allow` narrows to the named tools, `deny` drops from what remains, and a filter entry matching no advertised tool — or two tools that escape to the same Lua name — is a hard startup error the operator must fix. A server that simply fails to spawn is dropped with a warning rather than failing the boot.

## Model identity is not double-recorded

Which model or template produced an inference is already captured per-event in `produced_by`, so keeping the model endpoint environmental loses no replay fidelity: faithful replay uses stored outputs (model-agnostic), and regenerative replay reads `produced_by` to know what to re-run. The endpoint is just where to reach it.

## Build-default changes do not silently apply

The settings snapshot is pinned in the agent's own log, so when a zuihitsu build ships a new default for a tunable, existing agents keep theirs, exactly as with prompt templates: a new default reaches only agents born after it. Because genesis copies the build's current defaults into the agent's own log, the agent is thereafter independent of the build it was born from.

A genuinely new knob is the one exception, and it adopts its build default silently by construction: absent from every snapshot written before it existed, it deserialises to that default — the only value it could take, since a setting that didn't yet exist can't have been pinned. The operator changes settings by replacing the whole snapshot (`POST /control/settings` writes a fresh `ConfigSet` under operator source); `GET /control/settings` reads the current one.

## Prompt templates

The orchestration prompt templates live in the stream as `PromptTemplateRegistered { name, version, body, source }`, materialised into a `prompt_templates` table and read back as the highest version per name. The name is a closed, build-defined enum (`PromptTemplateName`): the current set is the system-prompt scaffold, description-regen, temporal-extraction, flush, imprint, and link-inference. They are orchestration config, not agent-editable: `source: Orchestration` at genesis (an operator edit registers under `source: Operator`), never `Agent`, so the agent cannot rewrite its own regen prompt via Lua. Updating a template is a new registration with a bumped version, and old `produced_by` references keep pointing at the old version. Templates follow the same build-independence as settings (see [Build-default changes do not silently apply](#build-default-changes-do-not-silently-apply)): genesis copies the build's templates into the agent's own log, so a new default reaches only agents born after it.

Prompt *content* is supplied by the build, not fixed by this document. Genesis ships build-authored templates whose wording is iterated over time, consistent with the API description being a function of the build rather than this document. The one fixed point: the entire judgement layer — sensitivity inference, "ask before writing," belief arbitration, and the third-party residual — is carried by the template wording, not by code.

The scaffold's body is additionally shaped at genesis by the instance's `InstanceFeatures`: `default_templates(&features)` drops the dotpoints that teach a disabled feature's practice, so the prompt never teaches something the agent cannot do (see [Instance features](../CONTRIBUTING.md#instance-features) in `CONTRIBUTING.md`). This baking is a genesis-time decision — the features set at rollout decide which guidance persists, and a later turn reads the baked template verbatim.

## Creation (idempotent, via the operator surface)

You provide a seed-self — a name for the agent, a one-line persona, and optionally a few seed disposition entries (`SeedSelf` in `src/agent/genesis/mod.rs`) — through the operator surface (the CLI's `create`, or the console, both reaching `POST /control/agent`), and genesis::rollout resolves the build's default templates, seed relations, and seed tags and rolls out the genesis sequence against a fresh log, committing the whole tail as one atomic append:

```
PromptTemplateRegistered (scaffold, vN)
PromptTemplateRegistered (description-regen, vN)
PromptTemplateRegistered (temporal-extraction, vN)
PromptTemplateRegistered (flush, vN)
PromptTemplateRegistered (imprint, vN)
PromptTemplateRegistered (link-inference, vN)
LinkTypeRegistered       (created_by / created)              -- historical origin (who made it)
LinkTypeRegistered       (operator_of / operated_by)         -- current operatorship (whose instance this is); distinct from created_by, so operatorship can transfer without rewriting origin
LinkTypeRegistered       (knows / known_by)
LinkTypeRegistered       (same_as / same_as)                 -- symmetric; cross-platform identity
LinkTypeRegistered       (participates_in / has_participant) -- a person's attendance at an event
LinkTypeRegistered       (part_of / contains)                -- membership or aboutness: an event, entry-bearing memory, or sub-topic belonging to a topic, project, or workstream (not people, who participates_in)
LinkTypeRegistered       (located_at / location_of)          -- placement: where a thing is held or found (an event's venue, a team's office, a thing's place)
TagCreated               (confidential)                      -- the one seed tag: marks a context confidential
ConfigSet                (default behavioural-settings snapshot, source Bootstrap — see Configuration)
MemoryCreated            (self)
MemoryContentAppended    (self <- the persona, then any seed disposition entries — the charter)
GenesisCompleted         { manifest_hash, template_versions }
```

The seed relations are a minimum-viable ontology of structural universals. `located_at` (placement) earns its seat because the agent otherwise re-coins it under scattered spellings — `held_at`, `occurs_at`, `based_in` — in crash-then-register cycles; that recurrence is the bar a relation must clear to count as a universal the system leans on. Social semantics — mentorship, employment, and the like — remain the agent's to coin with `links.register` as its environment calls for them (see [The seed ontology](../CONTRIBUTING.md#the-seed-ontology) in `CONTRIBUTING.md`). Genesis emits no `created_by` link and no facts about anyone; a freshly-born agent genuinely doesn't know who made it.

The teller of genesis content is a `Bootstrap` pseudo-teller, since no real participants exist yet, and the source of the registrations is likewise `Bootstrap`/`Orchestration`. Two reserved non-participant tellers exist: `Bootstrap` for genesis, and `Agent` for content the agent authors about itself or its own observations (see [Visibility → Defaults](visibility.md#defaults-at-write-time)).

The persona and any seed disposition are recorded as `self`'s content *entries* — the charter — not as its description, told by `Bootstrap` with public visibility. The system prompt draws the agent's identity from these entries verbatim (see [System prompt](conversations-and-briefs.md#system-prompt)), and because entries are immutable and append-only the authored voice never drifts, while the self still evolves as the agent appends further self-observations under the `Agent` teller. `self`'s description regenerates like any other memory's, but as a lossy summary for search and compaction, never as the source of the voice.

Creation is **idempotent**, so it doubles as "resume an interrupted genesis." Each genesis event has a content-stable dedup identity — templates on `(name, version)`, relations on `name`, tags on `name`, config on being present at all, and `self` on its unique name, never on freshly-minted ULIDs — so re-driving the sequence replays present events as no-ops and emits only the missing tail. `GenesisCompleted`'s `manifest_hash` is a SHA-256 over the seed-self (name, persona, entries) and the `(name, version)` template pairs, so it is stable across resumes and independent of minted ids. Calling creation on a born agent is a no-op (`Rollout::AlreadyComplete`). The seed tags and relations are build defaults rather than part of the manifest hash, so adding one does not perturb an existing agent's hash.

## Boot (every startup)

Boot first opens the event log in WAL mode and takes an **exclusive advisory file lock** on it (via `fs2`), held for the store's lifetime, and refuses to start if another process already holds it — one log, one writer, failing with "event log … is already open by another writer." This is what keeps the multi-agent-one-executable story (see [Configuration](#configuration)) from silently violating the single-writer invariant if two configs are pointed at the same storage directory: the second instance fails fast rather than corrupting the log with a second writer. Read-only inspection (`zuihitsu debug events`) opens the log without a lock, so it is safe while the agent runs; the destructive `zuihitsu debug revert` opens it read-write and so requires the agent stopped.

Before the graph is opened, boot restores it from the latest snapshot when that snapshot leads the on-disk graph (a fresh, deleted, or corrupt graph), so the subsequent catch-up replays only the log tail rather than the whole log (see [Snapshots](events-and-storage.md#snapshots)). Boot then catches the graph up to log-head by materialising forward from the store. This same forward catch-up reconciles a graph left behind by a crash in a commit window (see [Storage → Commit and boot span two stores](events-and-storage.md#commit-and-boot-span-two-stores)), so a half-applied commit self-heals. If the embedding model changed, boot clears and rebuilds the vector index before serving (see [Vector store](events-and-storage.md#vector-store)).

Boot then classifies the log by the presence of `GenesisCompleted`, **not** by log emptiness, because a crash mid-genesis must not be mistaken for a born agent. Three states (`GenesisStatus`):

1. *Log contains `GenesisCompleted`* → a born agent, caught up to head and ready to serve. Normal boot.
2. *Log empty* → no agent yet. The server still starts (so the operator can create the agent), but there is no `self` to converse against; the operator is directed to create the agent via the console.
3. *Log non-empty, no `GenesisCompleted`* → an interrupted genesis. Never treated as a born agent; re-running creation re-drives the whole sequence idempotently (present events no-op, missing ones emitted, the manifest hash stable across the resume). "Resume an interrupted genesis" is just "re-run creation."

Once the graph is caught up, boot seeds the link-inference cursor to log-head, so state written before this boot is treated as already processed and a restart does not re-run that pass over it. Serving then brings up the HTTP API and the background workers (scheduler, indexer, describer, link-inference, idle and checkpoint sweepers, and the snapshotter), each spawned only when its prerequisites — a configured model, an embedding endpoint, or snapshotting enabled — are present.

## Imprint interview (creator self-introduction)

Real self-knowledge forms in a console-launched imprint session — a genuine conversation, but one whose writes carry operator authority (`Authority::Operator`, sourced as `Operator`). The operator opens it (`POST /control/imprint`, or the CLI's `interact imprint`, one operator message per call) and simply talks to the agent through the reserved `operator/imprint` conversation, driven by the `imprint` prompt template. Because the session is console-authorised, these writes — including any to `self` — are permitted; because they are only permitted under that authority, no ordinary platform conversation can forge them, and this is the only conversation in which the agent may write `self`. The interview runs no compaction (it is short, and a flush would run barred from `self`), is multi-turn, and is re-runnable on demand from the console.

The operator is held provisionally as a reserved `person/operator` stub — minted once, on the first imprint, keyed only by its canonical name and carrying no platform binding, so it never collides with a real participant and resolves identically across imprints. The `imprint` template teaches the agent, once it learns the operator's real name, to create the canonical `person/<name>` memory (no platform suffix), record what it learns there, merge the provisional stub into it with `links.create(person/operator, "same_as", person/<name>)` so they are one identity, and assert `links.create(self, "created_by", person/<name>)`. `person/operator` holds no content — every fact about the operator lives on their real profile. The self-observations the agent records (purpose, disposition) go on `self` under the `Agent` teller.

"Who created you?" then answers from the agent's learned model surfaced in the self-brief: the `created_by` link is structural and public, so it shows up in `self.relationships` regardless of the description. The creator is introduced, not discovered from whoever spoke first, which retires the imprinting-as-injection vector entirely. There is no conversational self-write for a stranger to exploit, because conversational self-writes don't exist outside the console-authorised session. Because genesis is just events, the agent's autobiography is continuous from `seq 0`.
