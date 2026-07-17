# The agent loop

## Agent loop and tool protocol

A turn begins when a platform message is routed to the server (see [Server API and turn lifecycle](#server-api-and-turn-lifecycle) below); the loop described here is what runs inside it. Before the first step, the [ambient recall](conversations-and-briefs.md#ambient-recall) pass may append one recorded system hint after the inbound message — memories the frozen brief did not carry, surfaced lexically from the message's own words — so the loop starts aware of what it would not have thought to search for.

A turn is a loop of model *steps*. At each step the model is given the conversation so far and emits either tool calls or a final reply — never both in one step, because a reply composed before seeing a tool result would be reasoning on stale information. The contract:

- Tool calls use the model's structured tool-calling interface, not parsing out of free-form text. There is effectively one tool, `run_lua(script)`, whose argument is a Luau block; the structured call replaces any fenced-block parsing.
- A step may contain one or more `run_lua` calls. They execute sequentially in emission order, each as its own block and its own transaction (see [Lua API → Block transactionality](#block-transactionality)). All run against the session's one VM, minted fresh when the session opens and shared across every turn and every block of the session. A later call in the step therefore sees an earlier call's committed writes: each block commits to the store, and the next block reads the now-updated graph. Their rendered results are returned together and the loop steps again.
- Atomicity across operations is achieved by putting them in one block, not by emitting several calls. Several calls in a step are a convenience, not a transaction boundary. The model's tool calls run sequentially, and external I/O runs inside a block rather than as parallel tool calls: a block's MCP calls are synchronous, each blocking the block until the server answers (see [External I/O](#external-io-via-mcp)).
- A step with a final reply and no tool calls ends the turn, and the reply is delivered to the participants.
- A reply that leaks chat-template special-token markup — the `<|…|>` delimiters wrapping a backend's tool-call or turn tokens — is never delivered to a participant; it means the model transcribed template scaffolding into its answer, typically at the forced-answer final step where withdrawn tools provoke a weak model to spell out a pseudo-tool-call. The step resamples once and delivers the retry only if it comes back clean prose, otherwise it falls through to the stay-silent terminal rather than surfacing markup.
- A step may instead end the turn with no reply — an explicit *stay-silent* terminal, distinct from a reply. This is a first-class loop outcome, not prompt guidance layered over a loop that always emits: in a group room a message may not be addressed to the agent, and "say nothing" must be representable. A silent terminal still records a [`ConversationTurn`](events-and-storage.md#event-sourcing), so the log and console show the agent saw the message and chose not to answer (auditable silence, distinct from a dropped or unprocessed message), but it delivers nothing to the platform client.
- A per-turn `max_steps` bound caps runaway loops, and the loop makes the bound legible rather than letting the agent walk into it blind:
  - **Nudge injection.** A one-line system nudge is appended to the step frame — "two steps remain in this turn — finish gathering and answer with what you have." — riding the in-memory step frame as a trailing system message, never recorded to the log, so the model can spend its remaining budget on the answer rather than another search.
  - **Its trigger.** The nudge fires two steps out, and only when the bound is at least two steps.
  - **Tool withdrawal on the final step.** On the final step the tools are withdrawn (tool choice `none`), forcing the model to reply with what it has gathered; that reply terminates the turn on the ordinary path.
  - **The `max_steps` error.** Hitting the bound anyway — the model producing no text even when it can no longer call a tool — ends the turn with a surfaced error the agent can reason about next time, the fallback path, not the norm.
  - **One agent event per cycle.** Like the other terminals, whichever way it ends it records the cycle's single `ConversationTurn(role = agent)` — carrying the reply, or the surfaced error — so the invariant "exactly one `role = agent` event per response cycle, however it ends" holds for the reply, silent, and `max_steps` paths alike.

Each `run_lua` execution is recorded as a `LuaExecuted` event under the rules in [Event sourcing](events-and-storage.md#event-sourcing): what the agent saw is what's stored. The loop itself is orchestration, not agent-editable.

**A block reports what it committed, and the agent re-sees that across turns.** A write block's result carries a concise summary of the effects it committed — `Committed: created topic/q3_plan; appended 2 entries to topic/q3_plan.` — so a block that returns nothing still confirms to the agent that its create or append *landed*, rather than a bare `nil` that says nothing about whether the write took. Those committed-effects summaries then persist into the cross-turn conversation buffer (alongside the reply text, but **not** the within-turn scratch — the script, the query results, the step reasoning), so on a later turn the agent re-sees what it durably wrote. This distinction — the ephemeral scratch is hidden, the durable effects are carried — is load-bearing. An agent that cannot see its own prior writes mistakes a fact surfaced in front of it (sitting in the buffer, or recalled) for something new and re-issues the write. That re-records a [confidence](visibility.md#what-privatetoteller-actually-promises) under whoever is now speaking (silently re-keying whose private note it is) and re-dumps working state at every [flush](conversations-and-briefs.md#compaction-token-triggered-re-segmentation). Recording is for what is *new*; a fact already held needs no re-recording, and a question that merely surfaces something known is answered from memory, not written again.

## Server API and turn lifecycle

Clients reach the server through a small HTTP API; the server owns the loop, the log, the model, and the scheduler. The CLI does not open the store itself — only the running instance holds the single-writer log lock — so every operator command is a request to that instance. The surface splits into two sub-routers by client authority (see [Clients and the server boundary](trust-and-authority.md#clients-and-the-server-boundary)): the participant surface under `/platform`, and the operator surface under `/control`. Each sub-router carries its own bearer-key auth layer, so a control key never authorises `/platform` and a connector key never authorises `/control`. Everything not under those prefixes falls through to the embedded web console (its assets by path, `index.html` for any client-side route), served from the agent's own origin so it reaches `/control` keylessly as a loopback peer.

### Trust model on the wire

The control surface carries its own key list and the platform surface its per-connector keys, independent of each other, so a key issued for one surface never authorises the other. The full wire-auth rule — loopback trust, remote key-gating, and its fail-closed, constant-time enforcement — lives in [Clients and the server boundary](trust-and-authority.md#clients-and-the-server-boundary).

### Platform-client surface

Platform authority: deliver and receive, acting only as the represented participants. Every request is scoped to one connector, resolved from its key, which supplies the platform every operation acts on — so no payload carries a platform, only bare ids and a `scope_path` resolved under it, never operator authority. A loopback peer is the operator's own console, scoped to the reserved `direct` interface. The absence of the operator methods on this surface is what makes "the platform client has no operator authority" structurally enforceable rather than policy.

- `POST /platform/message` — the core call, `route_message`. The body is `{ scope_path, sender, text, present }`: the room the message arrived in, who sent it, the message, and who is currently present. `sender` and each entry of `present` is a bare id, resolved under the request's connector platform to `person/<id>@<platform>`. The server pairs the `scope_path` with that platform, resolves it to a conversation (minting its context memory on first contact) and each id to a participant stub (minting on first contact), opens or continues a session, appends the inbound `ConversationTurn(role = participant)`, runs the agent loop, and returns a `TurnOutcome`. It needs the model, so it answers `503` when no model endpoint is configured. It holds one shared-model stream permit for the whole handling — the turn and any compaction flush it triggers — so no more than `max_concurrent_streams` messages crowd the model at once (see [Concurrency](write-path.md#concurrency)).
- `TurnOutcome` is one of: a **reply** (text to post back), **silence** (the stay-silent terminal, nothing to post), a **`max_steps` terminal** (the loop hit its step bound without a reply — the outcome carries no text, but the surfaced error `max steps (N) reached without a reply` is recorded as the agent's turn for the agent to reason about next time), or a **deferral** (the model backend was unreachable — retries exhausted or the circuit open — so no response cycle ran; the inbound turn is durably recorded and catch-up is passive: the next inbound message replays the buffer, which includes every deferred inbound, so one response cycle covers them all — see [Transport resilience](write-path.md#transport-resilience)). The unary endpoint delivers the reply as one whole JSON `TurnOutcome`; a connector that wants the generation as it happens posts the identical body to `POST /platform/message/stream` instead, and receives server-sent events — the turn's reply (and reasoning) tokens as `progress` frames, then the whole `TurnOutcome` as the terminal `outcome` frame, identical to what the unary endpoint would have returned. A connector that ignores every `progress` frame behaves exactly like one that never upgraded; the frames exist for typing indicators and partial-message edits, are never stored, and a turn's failure arrives as a terminal `error` frame. The console watches the same generation through its own channel, `GET /control/events/stream` (below).
- `POST /platform/join` — `note_join`, the explicit path for a client that delivers presence changes as their own signal, separate from a message. The body is `{ scope_path, participant }`, where `participant` is a bare id resolved under the request's connector platform. If the room has a live session, it records a `ParticipantJoined` and injects the joiner's brief — built against the now-present set, so the [subject-guard](visibility.md) suppresses asides about them — as a `system` turn at the join point, rather than rebuilding the frozen prompt (see [Contextual briefs](conversations-and-briefs.md#contextual-briefs)). It is a no-op if the room has never been seen or has no live session; the next message then opens a session with the joiner present. It succeeds without a configured model (the join composes off the current prose rather than returning `503`); a configured model feeds the joiner's [describe catch-up](write-path.md#freshness-before-a-brief) before the brief composes.

- `POST /platform/roster` — `note_presence`, the resync counterpart to `/platform/join` for a connector that observes the whole roster directly (a voice channel's member list, a presence event) rather than one arrival at a time. The body is `{ scope_path, roster }`, the full set of bare ids currently present, each resolved under the request's connector platform. If the room has a live session, the server diffs the roster against its stored participants: each arrival takes the same path as `note_join` — a `ParticipantJoined` and an injected join-brief — while a departure is acknowledged but records no event, for the same reason the per-message sync leaves departures eventless (below). The response is `{ joined, departed }`: the bare ids it briefed in, and the count of prior members the roster no longer lists. It is a no-op returning an empty resync if the room has never been seen or has no live session; the next message then opens a session with the current roster present. Like `/platform/join`, it succeeds without a configured model, and a configured one feeds each arrival's describe catch-up first.

- `POST /platform/project` — `project`, for a connector to record platform attributes onto a scoped memory as ordinary public entries: a participant's identity (the username, display name, and nickname the platform surfaces) onto their `person/*` stub, or a guild's name onto its `context/*` memory. The body is `{ target, attributes }`, where `target` is `{ participant: { id } }` or `{ context: { scope_path } }`, resolved under the request's connector platform (the connector is the request's, from its key, not a body field); each attribute records a new value or clears one, superseding or retracting the entry a prior projection returned (the connector holds the ids, so the server keys nothing). The response is the new entry id per attribute. A connector projects only the entities it represents — never the agent's own bot identity, which is not another participant.

- `POST /platform/link` — `link`, for a connector to assert (or retract) a structural `part_of`-style link between two of its own scoped memories: a channel and its members placed in a guild, say. The body is `{ from, to, relation, remove }`, where `from` and `to` are each `{ participant: { id } }` or `{ context: { scope_path } }`, resolved under the request's connector platform. On assert a missing endpoint is minted; on retract the endpoints are resolved without minting, so a retract naming an unknown node is a no-op. The edge is `Public` and carries `LinkSource::Connector` (which connector authored it). `same_as` is refused — cross-platform identity is operator-adjudicated — and an unregistered relation is a `400`.

**Presence sync is per-message, not a separate endpoint.** Most clients only ever deliver messages, so `route_message` treats the message itself as the join signal: the server diffs the message's `present` set (plus the sender) against the live session's stored participants, and every newcomer gets the same treatment as the explicit endpoint — a `ParticipantJoined` plus an injected join-brief, appended before the inbound turn so the brief precedes the message in the buffer. A fresh session is a natural no-op, since its `SessionStarted.participants` already carry the full present set. Departures deliberately record no event: the per-turn visibility predicate evaluates against the message's own present set, so a departed participant stops affecting retrieval on the very next message, and the session's stored participants feed only join detection and the flush's present set — where a stale-inclusive set errs toward suppression, the safe direction.

### Turn trigger: who decides a message becomes a turn

Not every message in a busy room should run a full agent loop. The gating decision lives in the platform client, not the server: the client decides which inbound messages to `POST /platform/message` (@-mention, direct reply, DM, name-trigger, and so on) and which to drop or merely carry as context. The server runs a loop for everything routed to it, and the agent's stay-silent terminal is the second, finer filter, for messages the client forwarded but the agent judges aren't for it. Two filters, cheap then smart: the client avoids waking the model for obvious non-addressed chatter, and the agent declines the rest. How much surrounding context the client forwards for un-routed messages — so the agent isn't blind to the room between mentions — is a client-policy tuning knob, not a server contract.

### Control-client surface

Operator authority. The whole `/control` surface is loopback-trusted and remote-key-gated (see [Trust model on the wire](#trust-model-on-the-wire)), and every action it drives runs under operator authority — the imprint turn runs as [`Authority::Operator`], and the console-only merge and edit paths write as the operator. These are the operator-only endpoints a platform client structurally cannot reach. The CLI and the web console are both control clients.

The write and interview endpoints:

- `POST /control/agent` — create the agent, or resume an interrupted genesis; idempotent. Returns the `Rollout`.
- `GET /control/genesis` — whether an agent exists and is ready.
- `POST /control/imprint` — one operator message of the imprint interview. The body is `{ text }` — a single operator message, not the full `route_message` shape; the server runs it as a turn against the operator's own `operator/imprint` conversation, under the `Imprint` template and operator authority. This is the only path that may write `self`. Multi-turn and re-runnable; needs the model, so `503` if none is configured (see [Imprint interview](lifecycle.md#imprint-interview-creator-self-introduction)).
- `POST /control/merge` — the console's merge control (`resolve_merge`). The body is `{ from, to, accept }` (the two stubs by memory id). Accepting authors the merging `same_as` directly (`LinkSource::Operator`, the console-only path past the adjudicator); declining records the operator's refusal so the proposal settles. The console surfaces the adjudicator's pending proposals here for one-click resolution, but the same control also lets the operator assert a merge it knows to be true independently of any proposal — the operator is not limited to the agent's guesses. This is the operator's *dedicated* merge endpoint; there is no other one. It is not, however, the only way operator authority reaches a `same_as`: the imprint interview's committing Lua authors one (merging `person/operator` into the operator's real profile; see [Cross-platform identity](data-model.md#cross-platform-identity-operator-asserted-or-adjudicated)). The general operator Lua console (`POST /control/lua`) runs under the same operator authority — where a `same_as` create is honoured rather than rerouted to a proposal, as a platform block's create would be — but it is a no-commit sandbox, so it persists no merge.
- `POST /control/prompt` — register a new version of a prompt template. The body is `{ name, body }`; it replaces the template from the next read on, logged as an operator `PromptTemplateRegistered`.
- `PUT /control/settings` — replace the agent's behavioural settings, logged as an operator `ConfigSet`.
- `POST /control/snapshot` — write a graph snapshot now (the take-one-before-an-experiment trigger). `409` when snapshotting is disabled. The response names the file written, or is `null` when the graph was already checkpointed at its current head.
- `POST /control/lua` — run an ad-hoc operator Lua block in a no-commit sandbox and return its rendered result. The body is `{ script, allow_mcp }`; the block executes against the live graph (reads see real memory), but its buffered effects — including any `LuaExecuted` record — are discarded, so nothing persists. It runs under operator authority on a throwaway VM bound to a dedicated `console/lua` conversation. MCP is off unless `allow_mcp` is set and a host is connected, because an MCP call is a real external effect no sandbox can roll back (see [Observability → the operator Lua console](observability-and-testing.md#the-views)).

The read-only inspection surface the console and CLI use:

- `GET /control/health` — whether an agent exists yet, and the model transport's health (circuit-breaker state, consecutive-failure count, and the last failure's cause), which the console polls to drive its degraded-backend banner. `model` is `null` when no model endpoint is configured.
- `GET /control/config` — the environmental config this instance booted from (storage paths, endpoints, bind, snapshots, MCP servers), with secrets redacted by the types themselves (API keys as counts, MCP env as its variable names).
- `GET /control/metrics` — the runtime metrics as Prometheus text-format (a `?format=` query is accepted but ignored — Prometheus text is the only shape). The instance-derived gauges are refreshed from state on each scrape. `503` when the recorder could not be installed at boot.
- `GET /control/lua-api` — the structured Lua API catalogue the console renders as a reference guide.
- `GET /control/memory?name=` — a memory by name; `404` if it does not exist.
- `GET /control/memories?prefix=` — the live memories in a namespace, ordered by name.
- `GET /control/entries?name=` — a memory's local content entries.
- `GET /control/sessions?platform=&scope=` — a conversation's sessions, oldest first.
- `GET /control/recurring` — the memories carrying a recurring occurrence.
- `GET /control/arbitrations` — the recorded belief arbitrations, oldest first.
- `GET /control/merge-proposals` — the cross-platform merge proposals still awaiting the operator, in first-proposal order.
- `GET /control/interactions` — the recorded model interactions, oldest first (the deliberation surface: per-call request, reasoning, token usage, and latency).
- `GET /control/events?from=` — the event log from `from` onward, in order (the whole log when `from` is omitted). The live console seeds its replica with one `from=0` read (see [One client over several sources](observability-and-testing.md#one-client-over-several-sources)).
- `GET /control/events/stream?from=` — the same log as server-sent events: the catch-up from `from` replayed as `event` frames, then the live tail pushed as it commits, with ephemeral `progress` frames interleaved — the token-by-token reasoning and reply text of any in-flight turn, published unconditionally (delivery simply requires a subscriber). Progress frames never touch the store and the materialiser never sees one, so the recorded log — and replay — is byte-identical whether or not anyone watched; the terminal `ModelCalled` remains the one durable record of a model call. The console tails over this stream and falls back to polling `GET /control/events?from=<head + 1>` when the stream is unavailable.
- `GET /control/settings` — the agent's current behavioural settings.

State, events, conversation, and time-travel are the read surface the console folds into a replica (see [Observability](observability-and-testing.md#observability)).

### Session lifecycle is server-owned

A session opens on the first `route_message` to a quiet conversation and closes on an idle timeout the server tracks — the session-gap threshold, `idle_gap_seconds` (see [Known limitations](limitations.md)). `SessionStarted` / `SessionEnded` bracket it, and `SessionStarted` is what freezes the brief and stamps the participant set. The client does not manage sessions; it routes messages and reports presence. The server decides session boundaries, so they are consistent across clients and recorded in the log rather than inferred per client, and never recomputed at replay.

The server owns three ways a session ends, all through the same flush-then-`SessionEnded` path, each recording its `cause`: the background idle sweep closes a session gone quiet past the gap (`Idle`); a `route_message` whose peak prompt crosses the token budget ends the session so the next message re-segments with a fresh brief and a carried tail (`Compaction`, see [Compaction](conversations-and-briefs.md#compaction-token-triggered-re-segmentation)); and a cold-start `route_message` after a restart recovers a session still open in the log — resuming it untouched within the idle gap (an identical prompt prefix, so the serving cache survives the restart), or closing it with a flush past the gap (`Recovery`). No close stages anything for the next session: the reopen reconstructs the previous session's raw-transcript tail from the event log, so a reopen after a quiet gap resumes with the last few things said rather than opening cold on the transcript, and the tail survives a restart between the close and the reopen (issue #86). A substantive session gets one budget-gated pre-compaction flush turn to write durable working state before the cut; the flush runs before `SessionEnded`, so a flush failure leaves the session standing for a retry rather than dropping its state.

## Lua API

Thin, composable, discoverable, with errors that teach. Object-and-method style: operations live on the things they operate on. The language is [Luau, a sandboxed Lua dialect](#the-vm-is-a-sandboxed-luau).

### The tool call returns its last expression

Each invocation is a small script; the value of its final expression is handed back to the agent, REPL-style. `memory.search("climbing")` as the last line returns the results; a bare `dave:append(...)` returns whatever `append` yields. Side-effecting operations still emit their events regardless of what the script returns.

### One VM per session

The same VM serves every tool call across a session, so scope is meaningful: a `local` lives for one script, while a global persists across tool calls for the whole session — an ephemeral scratchpad for stashing a fetched page and referring to it in a later call. It does not persist across sessions: a durable conversation that spans months does not carry one ever-growing scratchpad, and each session starts fresh.

The VM's internal state is not event-sourced and not reconstructed on replay, and doesn't need to be: anything the agent saw came back as a stored `result`, and any side effect a global produced was emitted as a concrete event payload. The scratchpad is working memory within a live session; anything worth keeping must be written to memory, which is event-sourced. The VM is working memory; the event log is long-term memory.

### The VM is a sandboxed Luau

Blocks run on Luau, chosen for its sandboxing-first design. Only the pure standard libraries (`string`, `table`, `math`, `utf8`, `coroutine`) are loaded, so `os`, `io`, `package`/`require`, `debug`, and the dynamic code-loading globals are absent: a block is an orchestration script over the projected API, never a host program, and MCP is its only sanctioned outward reach. Dropping `os` also keeps a block deterministic under replay, since the only time it can read comes from the injected clock. Luau already omits the code-loading globals (`load`, `loadstring`, `dofile`, `loadfile`) and has no `require`, but they are cleared to nil defensively before the freeze, so a later addition of one cannot reopen the sandbox. Once the environment is installed the sandbox is frozen — the global table and the stdlib are read-only to scripts — so a block cannot monkey-patch the libraries or smuggle state past the per-session scratchpad, while the host-side installs still write their globals freely.

Two host-installed helpers ride the frozen surface. `print(...)` does not write to a process stdout the model never reads; it captures into the block's output buffer, rendered the same way a returned value is, so the agent sees what it prints fed back (tab-separated arguments, newline-terminated, matching Lua semantics). `inspect(value)` is a pretty-printer for tables, useful for eyeballing a result's structure. Both are installed before the sandbox freezes so they are part of the read-only surface, but `print` is re-installed per block because it binds the current block's output buffer.

### Block transactionality

A `LuaExecuted` block is an atomic transaction over the event log. Side-effect events (`MemoryContentAppended`, `LinkCreated`, `TagAppliedToMemory`, and so on) are buffered during execution and emitted atomically at commit, all sharing the block's `turn_id`. If the block doesn't commit, the buffer is discarded and no side-effect events reach the log. This is what makes the timeout-abort-and-retry backstop safe: a retry isn't re-emitting events, because the first attempt emitted none.

- *Read-your-writes within a block.* Buffered side effects are visible to reads from the same block: `dave:append("X")` then later `dave:entries()` sees "X." Other conversations see the writes only at commit, all at once. Mutex scope aligns with transaction scope: other conversations can't see partial writes because they can't acquire the locks, and at commit they see everything atomically.
- *Commit is per-block, not per-turn.* Multiple blocks in one turn each commit on their own boundary; a later block in the same turn reads an earlier block's writes through the materialised graph, not through buffer isolation.
- *Explicit abort: `block.abort(reason)`.* A clean lever to discard a block's buffered writes mid-script, better than raising an error. It's an agent-visible terminal outcome (the agent did it deliberately and reasons about it next turn), so it emits a `LuaExecuted` with `result: null` and `terminal_cause: aborted("reason")`. Runtime errors emit similarly. The console conversation view surfaces aborts and errors distinctly from successful blocks.

### The API description is injected into the system prompt and is deliberately not versioned

The catalogue of functions — signatures, examples, the connected MCP servers' projected tools, the current tag vocabulary, and registered relations — is rendered into the system prompt so the agent always knows what it can call. The MCP tools are runtime-derived, from whichever servers are connected at assembly time, rather than build-derived, but fall under exactly the same not-versioned, additive-only discipline.

This is an intentional asymmetry with prompt templates, which are versioned in the log: the API description is a function of the running build, reflecting what the binary actually provides, and versioning it in the log would risk drifting from reality. Faithful replay is unaffected, since it feeds back the captured frozen prompt the agent saw (see [Faithful replay](events-and-storage.md#faithful-replay)); the additive-only, backwards-compatible discipline matters only to regenerative replay, which rebuilds the prompt under the current build. There the MCP slice is doubly non-faithful — build drift plus the external-server drift of whichever third-party servers happen to be connected at replay time — which no versioning could prevent.

The API surface is per-instance configurable via `InstanceFeatures` — a bitfield (`linking`, `tagging`, `merging`, `calendar`, `transcripts`, plus always-on `memory` and `context`) set at construction. A disabled feature is dropped from three gates in lockstep: the Lua functions are not installed (calling them is a nil-call error), the API-description entries are omitted, and the scaffold dotpoints that teach the practice are dropped from the baked template. The scaffold is baked at genesis, so feature-gating it is a genesis-time decision; the Lua registration and API description read the running binary's features fresh each turn. See `CONTRIBUTING.md` → Instance features.

### Memory operations

```lua
-- Module-level
local mem = memory.create("person/dave", "Met at the climbing gym")
-- The optional third argument carries the same overrides as :append, so a reminder can be
-- created and timed in one call (occurred_at is a TemporalRef, see Time).
local ev = memory.create("event/standup", "Team standup", {
  occurred_at = { recurring = "FREQ=WEEKLY;BYDAY=MO" }, visibility = "public" })
-- (the content argument is recorded as the first appended entry, not stored on
--  the MemoryCreated event — see Event sourcing; one provenance path for all content)
local dave = memory.get("person/dave")   -- resolves by name or a handle; nil when nothing resolves
local same = memory.get_or_create("person/dave", "Met at the gym")  -- fetch, or create if absent
local hits = memory.search("climbing", { tags = {"hobbies"}, namespace = "person", limit = 5 })
local stems = memory.list("person/")     -- discovery by handle prefix, alphabetical
local stub = memory.get("person/dave@discord")   -- a stub name resolves to that one stub, not the class

-- Methods on Memory objects
dave:append("Dave got a new job at Hooli")
dave:append("Got a new job", { occurred_at = "last week", visibility = "private" })
dave:tag("colleagues"); dave:untag("strangers")
dave:supersede(old_entry, new_entry)
dave:revise(old_entry, "Now a staff engineer")   -- append the new value and supersede the old in one atomic call
dave:rename("person/sarah")   -- same memory, new handle: when someone changes the name they go by
dave:set_volatility("high")   -- how fast this memory's facts age (high | medium | low; see Time → decay)
dave:entries(); dave:history(); dave:details()   -- live entries; full history; the whole record in one string
dave:propose_merge(memory.get("person/dave@discord"), { rationale = "same climbing-gym Dave" })

-- Link writes (the links.* module, not a handle method); the arguments read as a sentence
links.create(dave, "works_at", memory.get("company/hooli"))   -- a One-cardinality relation replaces in place
links.create(dave, "knows", "person/erin")   -- the object may be a name string as well as a handle
links.create(dave, "knows", "person/erin", { visibility = "attributed" })  -- opts.visibility forces the posture
links.remove(dave, "knows", "person/erin")

-- Link readers (auto-traverse same_as); each result renders as "relation → name"
dave:outgoing("mentors"); dave:incoming("mentors"); dave:links()
```

`memory.create(name[, content][, opts])` mints a new memory and, when `content` is given, records it as the first appended entry; it raises if the name already exists, and that strictness is load-bearing — the merge and identity flows rely on a second stub over a name being a deliberate act. `memory.get_or_create` is the idiom `memory.get(name) or memory.create(name, ...)` folded into one call for when existence is uncertain; when the memory already exists it is returned as it stands, `content` and `opts` ignored, so a fetch never silently overwrites what is recorded. `memory.get` returns nil when nothing resolves, and accepts either a name string or an existing handle (a handle resolves by its current name, so the lookup is identical either way); a renamed person still resolves by a former name, and a handle resolved by a former name carries `former_names` and `former_handle` fields plus an active rename note into the agent's own output, so an old-name lookup is never mistaken for a second person. `memory.search(query[, opts])` recalls by meaning (semantic plus lexical, visibility-filtered against who is present); `opts` takes `tags`, `namespace`, and `limit` (default 8). It returns a list of result objects — `{ name, description, score, marker?, snippet?, occurred_at?, relations? }`, best first — each of which is also a usable memory handle (any handle method or lazy `name`/`description` falls through to the handle metatable, so `hits[1]:append(…)` works without a `memory.get` round-trip), and each of which prints as a readable line. `memory.list(prefix)` is discovery by *stem* — which spellings of a name already exist — where search is recall by meaning; the prefix is required and matched literally (its `%`/`_` do not wildcard), and the result is capped at 50 handles with the remainder noted in the rendered form. `mem:details()` renders the memory's whole record in one string (header with former names, every entry, links in both directions, tags, and volatility), reusing each dedicated reader's rendering; it is always installed, its link and tag sections simply empty on an instance without those features. `mem:propose_merge(other[, opts])` records that two memories may be the same person across platforms for the adjudication pass to weigh — not a merge, and it surfaces nothing until adjudicated; `opts.rationale` is weighed as the proposer's claim, not as evidence.

`mem:revise(old, new_text[, opts])` collapses the common correction — append the new value, supersede the old — into one atomic call: if the supersede fails because the old entry is not live, the append rolls back with it, so a correction never half-applies into a new value standing beside the stale one. A relation's subject or object on `links.create` and `links.remove` may be given as a memory's name string as well as a handle, resolved to its memory like any name.

A search hit is a **candidate, not a match**, and two guards keep a fuzzy recall from committing against the wrong referent. First, a hit carries the query it came from: a content or identity write through a hit — `:append`, `:supersede`, `:revise`, `:rename`, or a `links.create` endpoint — commits only when some whitespace- or punctuation-delimited token of that query exactly equals the handle's name segment (the part after `namespace/`). A stem proves nothing: searching "Davina" and landing the `person/david` hit whose entry merely mentions her does not pass, since no token of "Davina" equals "david". Reads through a hit stay free — only a write is gated. The refusal names the three ways forward: `memory.get` to confirm it really is them, `memory.list` on the shared stem to see who else lives there, or `memory.create` for someone new.

Second, and because a block is composed in full before any of its searches run, a search additionally *taints* — for the life of that block — the names of the hits its query did not name. A write to a tainted name through **any** handle in the same block is refused, not just a write through the hit itself: fetching the mismatched hit by name (`memory.get(hits[1].name)`) and writing through that fresh handle would slip past the per-hit guard, but the target name is tainted, so it is caught all the same. This closes the structural hole — an in-block `if #hits == 0 then … else hits[1] …` branch is judgement-free by construction, since the model reads nothing between composing the branch and the search running. The refusal points at the block boundary: return what you found, then decide in the next block, which is composed *after* the results are visible and carries no taint, so the same fetch-and-write passes freely there. The taint dies with the block; a legitimate same-block write to a memory a mismatched search also surfaced is the accepted cost, refused once and landing on the retry.

`same_as` is auto-traversed on reads: `memory.get`, search, and the link readers (`outgoing`/`incoming`/`links`) surface content and links from the whole class, deduplicated, with per-stub provenance preserved. A link reader orients every edge against the queried identity — `outgoing` for an edge the identity is the source of, `incoming` for one it is the target of — and surfaces only relationships pointing *out* of the class, never the `same_as` edges holding it together; each result carries the far memory as an actionable handle alongside the relation, direction, source, the teller who asserted it (`told_by`), and the far memory's representative occurrence (its freshest dated entry's `occurred_at`, authored outranking extracted), so a neighbourhood rendered from a hub keeps each spoke's *when* without a second read. Like the relation-registry reads, they reflect committed state, so a link written in the same block is not yet visible to a read in it. Writes are not traversed, so `dave@discord:append(...)` writes the Discord stub. A write through a class-spanning handle resolves to the class's primary stub, the right home for a platform-agnostic human-fact; to attribute to a specific platform, name the stub directly, `memory.get("person/dave@slack")`.

Visibility on append is given in the options table. Omit it for the write-time default (`Public` on your own memory, `PrivateToTeller` on someone else's); `visibility = "public"` → `Public`; `visibility = "attributed"` → `Attributed` (visible like public but carrying a `[via teller]` provenance marker — the middle posture for an ordinary relayed fact, see [Visibility versus disclosure, and three postures](visibility.md#visibility-versus-disclosure-and-three-postures)); `visibility = "private"` → `PrivateToTeller`; `visibility = { exclude = { "person/dave", erin } }` → `Exclude(set)`, with members named as handles or as Memory or participant objects.

The teller rides in the same options table. By default an entry is told by the current speaker; `by_agent = true` records it as the agent's own observation (the `agent` teller) instead — the right stamp when the agent, not the speaker, is the source, and what the unknown-teller error teaches when the agent tries to name itself through `told_by`. `told_by` — a person handle or a name string, looked up like a link target — attributes the entry to a specific teller, overriding both, so a relayed claim ("X said that …") is stamped with X as its source rather than the person relaying it, and a deferred or cross-turn write carries who the fact actually came from. An unknown `told_by` name is a teachable error, never a silent mis-attribution.

Reads render an entry **self-describingly**, prefixed by what governs reading it — when the fact occurs (if dated), whether it is contested, its visibility, and who it came from: `mem:entries()` prints `[private · from person/erin] …`, a dated fact reads `[2027-03-15 · public · from you] …`, a fact under an unresolved arbitration reads `[disputed · public · from person/erin] …`, and an aged fact on a `High`-volatility memory reads `[stale — no newer entry · public · from you] …` (the segments compose, so a dated contested confidence carries all of them), mirroring the inline marker search hits already carry (see [Visibility → Search is a third visibility surface](visibility.md#search-is-a-third-visibility-surface)).

The read also exposes fields a script can branch on: `entry.occurred_at` (the occurrence as the *same* tagged table `append` takes — `{ day = "…" }`, `{ recurring = "…" }`, etc. — so a read round-trips to a write and a script matches on `entry.occurred_at.day` rather than reparsing a string), `entry.visibility`, `entry.told_by`, and `entry.disputed`. The occurrence renders faithfully to its `TemporalRef` — a day as the date, a recurrence as its rule, a relative anchor as `after event/…` — so a recurring or vague occurrence is not flattened to a single instant.

Rendering the date into the entry text is what keeps it findable: a date held only in structured `occurred_at` is invisible to lexical search, stranding an agent that hunts for it by text.

So an agent reading a person's entries sees at a glance which are confidences to hold and whose they are, and which facts are contested, rather than bare text whose provenance it must reconstruct separately — which is what lets it honour a confidence it surfaces *from memory* (recalled on a later turn, in another room), not only one fresh in the conversation. It also lets it surface a disagreement it reads back, rather than asserting one side as settled.

The `disputed` marker is projected from the latest unresolved `BeliefArbitrated` (see [Write path → arbitration](write-path.md#coalesce-then-regenerate-once)), so it tracks the current state: it appears when neither account is credited and at least two competing entries are still live, and clears once a side is credited or one account is superseded.

### Tag operations

Creation and application are deliberately distinct: applying never mutates a tag's description; creating always forces a purpose.

```lua
tags.list()                                   -- [{name, description, count}]
tags.create("hobbies", "Recreational activities and interests")
tags.describe("hobbies", "Updated description")
dave:tag("hobbies")                           -- errors if missing, suggests near matches
```

### Link relation registry

```lua
links.register({ name="reports_to", inverse="manages",
                 from_card="many", to_card="one", symmetric=false, reflexive=false,
                 description="reporting line" })
links.list(); links.get("reports_to")
```

Registers one relation accessible under either label; the inverse view's cardinality is computed. Cardinalities are the lowercase strings `"one"` and `"many"`, parsed at the block boundary; `symmetric`, `reflexive`, and `description` all default (to `false`, `false`, and empty). `links.list()` returns the whole registry (each result `{ name, inverse, from_card, to_card, symmetric, reflexive, description }`, printing as `name / inverse — from-to[, symmetric][, reflexive]: description`), and `links.get(name)` returns one relation by either label, or nil.

### External I/O via MCP

The agent has two outward reaches: reading a web page, which is first-class and in-house through `web.markdown`, and everything else, which is through MCP (Model Context Protocol) servers the operator configures.

#### Reading the web: `web.markdown`

Fetching a web page is common and general enough to be a built-in rather than an operator-configured server, so it is a first-class Lua call: `web.markdown(url)` fetches an http/https page and returns its main content as Markdown. The fetch is a pipeline — pull the page over HTTP, extract the article with a readability pass (dropping nav, sidebars, cookie banners, and footers), render that to Markdown under the page's title, and truncate to a character cap — so the agent receives readable prose, not raw HTML or a page of chrome. Everything is in-house (`src/web/`): a reqwest transport behind a `WebFetcher` seam (so tests and the eval inject a fake), the pure extraction pipeline above it, and the tunables (`WebSettings`) for the timeout, byte cap, Markdown cap, user agent, and the private-address gate.

The transport carries a server-side request forgery guard: before connecting, and again on every redirect hop, it refuses loopback, private (RFC 1918), link-local, and unique-local addresses, unless `allow_private_addresses` is set. This is not theatre — the instance's own control API listens on localhost, so an agent-driven fetch to a private address would be a confused-deputy hazard against its own host. A non-HTML response, a bad status, a timeout, a refused address, or an oversized body each comes back as a catchable, teachable error the agent can adapt to. A GET is idempotent, so — unlike an MCP call — `web.markdown` does not latch the block's "made an external call" flag, and a fetch-only block that times out on a lock-wait stays retryable.

`web.markdown` is gated on the `browsing` [instance feature](../CONTRIBUTING.md#instance-features): with it off, the call is absent (the standard nil-call error), the API reference omits it, and the scaffold does not teach it.

#### Operator-configured capabilities: MCP

Everything beyond reading a page — driving a stateful browser, calling a tool, querying a source — is an MCP server the operator configures. The integration projects each server's tools into the Lua API as `mcp.<server>.<tool>{ ... }`: one function per tool, taking a single named-argument table and returning the result.

```lua
-- a stateful browser server: navigate loads the page, later calls reuse it
mcp.browser.navigate{ url = "https://example.com" }   -- or the keyword-escaped goto_{ ... }
local md   = mcp.browser.markdown{}            -- reads the page already loaded
local urls = mcp.browser.links{}
```

#### Tool names are escaped into valid Lua

A tool name that collides with a Lua keyword takes a trailing `_` (the `goto` tool → `mcp.<server>.goto_`); characters illegal in a Lua identifier are mapped to `_` likewise. The escaped name is what the system prompt advertises, so the agent always sees the callable form.

Each advertised tool yields exactly one function, so an alias is a second function: a server exposing both `goto` and its alias `navigate` makes both `goto_` and `navigate` callable, with no dedup. If two tools on one server escape to the same Lua identifier, that is a hard startup error — the operator must rename or `deny` one — rather than a silent shadowing.

#### Per-session server instances

The tool surface is session-stateful: `navigate` loads a page the later calls read, and the interaction tools (`click`, `fill`, `scroll`, `findElement` by backend-node id, and the rest) only mean anything against the currently-loaded page. So a server instance is owned by the session VM (see [Lua API](#lua-api)), with the same single-threaded, per-session lifetime as the agent's scratchpad. The VM host keeps a lazily-built `server → instance` map, spawned on first `mcp.<server>.*` use in the session (most sessions never browse, so most never spawn anything), and torn down when the session ends — an idle-gap close or a compaction re-segment — by closing the subprocess's stdin, waiting, then killing on a grace timeout.

Because the VM runs its blocks one at a time, that map is accessed serially by construction: there is no intra-session race. Concurrent sessions are necessarily of different conversations, since a conversation's own sessions are serial windows, and they get separate VMs, hence separate instances, hence separate browsers, with no shared page to clobber. Page state therefore does not survive a session boundary — a new session re-spawns lazily and the agent must re-`navigate`, exactly as the scratchpad doesn't persist. The `server → instance` map is pure runtime state, never in the log (a subprocess handle is not a fact about the agent, consistent with the [no-capture boundary](#no-capture-of-external-io-a-deliberate-replay-boundary)); an agent restart drops every instance, and the next session re-spawns lazily with page state lost.

#### Calling: arguments, results, and errors

A call blocks the block until the server answers; calls are synchronous — there is no promise API. The Lua argument table is marshalled to the tool's JSON-RPC `arguments` by a fixed rule: a table with consecutive integer keys from 1 becomes a JSON array, otherwise a JSON object (an empty `{}`, the no-argument case, is an object, since tool arguments are always a top-level object); integer-valued numbers serialise as JSON integers (so `timeout = 10000` is not `10000.0`); strings and booleans pass through. We do not re-validate against the server's `inputSchema`: the client is a pass-through and the server validates, surfacing something like `-32602 Invalid params` as a catchable Lua error rather than duplicating (and drifting from) the schema.

The result projects back by a fixed rule too. A result that is all text blocks with no `structuredContent` returns a bare Lua string, the text blocks joined with `\n` — the common case, where `markdown` returns one block. Anything else — a non-text block, or `structuredContent` present even alongside text — returns a table `{ content = { <block>, … }, structured = <decoded structuredContent or nil> }`. A text block is decoded explicitly to `{type="text", text=…}`; every other block type is carried through *verbatim* as the server's own JSON (only `text` is decoded on the wire), so an image or resource block keeps whatever keys the server sent (`{type="image", data=…, mimeType=…}`) rather than a renamed shape.

A JSON-RPC protocol error (unknown tool, dead subprocess, malformed call → e.g. `-32601 Tool not found`) or an `isError: true` result raises a catchable Lua error, so the agent can `pcall` and adapt rather than abort the whole block; a returned value is therefore always a success result. The honest caveat, confirmed against a real server: some failures arrive as ordinary content rather than as an error — for instance a browser server returning a DNS failure as an `isError: false` text block (`# Navigation failed / Reason: …`). The projection cannot detect that, so the scaffold instructs the agent to read results critically rather than assume a non-error result means success; we do not normalise what a server chooses to put in its content.

#### No capture of external I/O: a deliberate replay boundary

Tool results are not recorded in the log. The block's effects (its `MemoryContentAppended` and other events) are ordinary log entries and replay faithfully, so state is always reconstructible, but the fetched content that drove those writes is not captured. This is the same hard boundary any external I/O has, and rather than pretend otherwise we accept it: regenerative replay of an MCP-touching block re-runs the call (non-deterministic, since the page may have changed or gone), and the audit trail cannot show exactly what the agent read.

This also breaks the usual block-retry safety argument that an aborted block emits nothing, so a retry is invisible (see [Concurrency](write-path.md#concurrency)). Once a block has made an MCP call, the external effect has already happened and cannot be rolled back, so a block that has performed external I/O is not silently auto-retried on a lock-timeout abort — the `mcp` session latches a "made a call" flag on each call and resets it at the start of each attempt, so the turn machinery knows a timed-out block touched MCP. The timeout instead surfaces as a catchable error for the agent to handle. There is no `notifications/cancelled` sent: the in-flight call is simply abandoned (its response stream is now desynced), the subprocess instance is marked dead and dropped from the session's `server → instance` map — which closes its subprocess — and the next call to that server spawns a fresh one, with page state lost. Both losses are recorded in [Known limitations](limitations.md).

#### Bare-minimum host

The agent is an MCP client over stdio, with the server as a subprocess launched as argv (never shell-split), its stderr discarded and `kill_on_drop` set. Spawn is: launch the process, `initialize` (advertising a supported protocol version — the client currently speaks only `2024-11-05` — and no `sampling` / `elicitation` / `roots` capability), send the mandatory `notifications/initialized`, then `tools/list` once to snapshot the catalogue and build the `mcp.<server>.*` projection — all bounded by a 30-second init timeout (distinct from the per-block timeout) after which the spawn is declared failed. `initialize` is a negotiation, not an assertion: the server echoes back the protocol version it will actually speak, which may differ from the one advertised, so the client checks that returned version against the set it supports and declares the spawn failed — same as a timeout — if it can't speak it, rather than proceeding to talk past the server. Then `tools/call` on demand, each under a 60-second call timeout (a call that overruns marks the instance dead), and `shutdown` closes stdin, waits a 2-second grace, then kills.

The tool catalogue is probed once up front (a startup spawn, `tools/list`, `allow`/`deny` filter, then the probe instance is shut down), so the per-session Lua projection is a pre-built table of functions and the system-prompt rendering derives from the same filtered set; the live server *instance* is still spawned lazily on first actual use.

"Bare minimum" still owes the protocol a few obligations. A server-initiated request (a server reaching back for sampling or similar) is answered with `-32601 Method not found` and execution continues, never blocked waiting on it, which would deadlock. Server notifications are ignored, including `tools/list_changed`, since the catalogue is snapshotted at spawn and the prompt is frozen per session anyway. The instance is considered dead on subprocess exit, stdout EOF, a failed write, or non-JSON output on its stream, which drops its tools (see [Projected into the system prompt, and dropped when unavailable](#projected-into-the-system-prompt-and-dropped-when-unavailable)).

Configured servers are environmental config (the `[mcp.<name>]` block — see [Configuration](lifecycle.md#configuration)), so they are operator-chosen and therefore operator-trusted, the same posture as the rest of the system. The projection is general — adding a server is a config entry, not code — so a headless-browser server for stateful page interaction, or any other capability, is a config entry rather than a build change. A network-capable MCP server's egress floor (blocking private-network and loopback ranges) is set in its own launch config, where such flags live; the in-house `web.markdown` fetcher carries its own such guard (see [Reading the web](#reading-the-web-webmarkdown)).

#### Projected into the system prompt, and dropped when unavailable

Each connected server's catalogue is rendered into the system prompt's API description block (runtime-derived; see [System prompt](conversations-and-briefs.md#system-prompt)) as one entry per tool: the escaped Lua call form, then each argument as `name: type [required] — description` with small enums inline, plus the tool's own description. This is compact enough to bound the token cost of a ~20-tool server and detailed enough to call correctly without a round-trip.

If a server fails to spawn or dies, its tools are dropped from the system prompt so the agent is never told about a capability it doesn't have, and a call against an unavailable server raises a Lua error, so the agent learns in-band that it can no longer rely on it, the same way it would handle any tool failure.

#### Allowlisting tools and resources

A server can expose far more than a given deployment wants, both for prompt economy and for least privilege: a read-only research agent has no business with a JavaScript-`evaluate` tool or page-mutating click/fill tools. So `[mcp.<name>]` carries optional `allow` / `deny` lists, matched against the raw MCP tool name (the name the operator reads in `tools/list`, before Lua escaping), case-sensitively. With neither, the whole catalogue is projected; the filter is full-list → intersect `allow` (if present) → subtract `deny`.

The filter is applied once to the server's advertised surface, and both the Lua projection and the system-prompt catalogue derive from that same filtered set, so the agent is never shown a tool it can't call, nor handed one it isn't shown; a filtered-out tool has no `mcp.<server>.*` function. An `allow` or `deny` entry that matches no advertised tool is a hard startup error, not a silent no-op: a server that renamed or dropped a tool must force the operator to reconfirm the policy rather than let the agent's toolset change invisibly underneath a stale list. The projection covers tools; the `allow` / `deny` shape is the same for any projected surface.

### Context and the calendar

```lua
context.current()           -- the context/* memory for this conversation, or nil if there is none

-- Calendar queries: each returns a list of memory handles, soonest first (calendar feature)
calendar.upcoming("7 days"); calendar.upcoming({ within = "7 days" })   -- bare string or { within = ... }
calendar.overdue("3 days")                     -- concrete occurrences already past
calendar.on("2026-06-03"); calendar.on(calendar.today())   -- accepts a date object or a "YYYY-MM-DD" string
calendar.recurring()                           -- the memories with a recurrence rule

-- Date construction: the runtime does the arithmetic, so a date is never math the model carries
local today  = calendar.today()
local friday = calendar.next("friday")
local soon   = calendar.in_days(3); local later = calendar.in_weeks(2)
local exact  = calendar.date("2026-06-03")
-- Date objects are { day = "YYYY-MM-DD" }, so they double as occurred_at values and compose in text
local when   = friday:add_days(1):add_weeks(1):add_months(1)  -- calendar-correct, returns new date objects
friday:weekday(); friday:to_string()           -- "friday"; the ISO day as a string
```

`context.current()` resolves this conversation's [`Namespace::Context`] memory (its `#confidential` tag tells the agent whether the room is confidential), or nil when there is none. Unlike the brief's `<upcoming/>` block, the `calendar.*` readers are the agent's own queries and are not visibility-filtered — like `mem:entries`, the agent sees its whole memory. The date constructors and date-object methods are synchronous (they read the injected clock and do pure date math, touching no memory), returning date objects that print and concatenate as `"YYYY-MM-DD"`; a date object is read-only, so its arithmetic returns a *new* object rather than mutating in place.

### Transcripts: resolving a turn link

When the `transcripts` feature is on, `convo.turn(id)` resolves a conversation turn reference — the ULID carried in a `[turn:<ulid>]` token, the canonical agent-facing form — to that moment and a small window of the surrounding turns (three before and three after) in its session. A pasted console deep-link's `?turn=<ulid>` never reaches here: the connector normalises any URL to the token before the message reaches the agent (see [The connector contract](../CONTRIBUTING.md#the-connector-contract) in `CONTRIBUTING.md`), so this resolver reads a bare ULID and nothing more.

```lua
local exchange = convo.turn("01J...")   -- { id, ref, text, speaker, role, at, window }
```

The result carries the focal turn's fields at the top — `ref` is the canonical `[turn:…]` to cite it by, `role` is one of `participant`/`agent`/`system`, `at` a formatted timestamp — and `window` is the ordered surrounding turns (the focal one included, flagged `focused`). It prints as a readable transcript excerpt with a `»` marker on the focal line, so `return convo.turn(id)` reads back as the exchange. Resolution obeys the audience rule: a moment resolves only when everyone present here was in its audience. A malformed id, an id whose moment the present audience did not all share, and an unknown id are three distinct teachable errors; resolving is read-only and takes no lock.

Errors return structured suggestions (`"trvel" not found; did you mean "travel"?`); the agent learns its environment by tripping over it.
