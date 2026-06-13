# The console — build plan

The **console** is the operator's web interface to the agent: state inspection, control actions, and — its first incarnation — the **eval-package viewer**. This is the build-level companion to the design spec, and it deliberately does not restate the spec. Read the spec for *what* to build and *why*; read this for the architecture, the stack, and the mechanics.

- **What and why:** `docs/spec.md` → **Observability** (the four views, the access model, the phases, the Japandi aesthetic), **Validation and the eval harness** (what an eval package is, the judge, gating vs. metric), **Event sourcing** (the payload vocabulary), **Storage and materialization** (the store seam, snapshots, the two replay modes), and **Clients and the server boundary** (authority roles, the `/control` surface).
- This directory (`console/`) is the frontend's home. Nothing is scaffolded yet beyond the generated types in `src/types/`.

## The architecture in one paragraph

The console is a **materializing read replica**. It never asks anyone else what the state is: it obtains the event log from a *source*, folds it through **the agent's own materializer compiled to WASM** (real Rust, real SQLite bundled in the module), and renders every view as a query against that local graph. A source is anything that can deliver the log — an eval package's embedded per-run `Event[]` today, a live agent's `/control` event stream later — and everything above the source is shared by construction. This is the spec's "one client over two sources" promise made structural: the eval viewer and the live debugger are the same client, and the reconstruction layer is the agent's own projection code, not a parallel implementation that can drift.

## Why this shape (decisions and rejected alternatives)

**Why the real materializer and not a TypeScript reimplementation.** The console exists to answer "what was the agent thinking." A TS re-fold of the events shows a *second opinion* about what the events imply, not what the agent's projection logic actually produced — every divergence is a lie in the debugger, and every new event type or payload version would need handling twice, forever (the spec leans hard on additive schema evolution, so this tax compounds). With the materializer compiled to WASM, the console renders the same fold the agent lives in, by construction. The one thing a second implementation would have bought — an accidental N-version check that could catch handler bugs by diverging — is a bad way to buy verification (you can't tell which side is wrong), and the spec already designates the eval harness as the materializer's backstop.

**Why SQLite bundled in, not a pure-core carve-out.** The materializer is not a pure fold; its handlers are written against rusqlite, with FTS5 riding along. The earlier idea — carve a pure projection core out, make the SQLite layer a dumb applier, equivalence-test the two paths — works, but it is invasive surgery on the most correctness-sensitive subsystem in the system. Bundling SQLite into the WASM module makes that refactor unnecessary: the handlers compile as-is, there is only ever one applier, and FTS5 plausibly comes along for free. rusqlite landed `wasm32-unknown-unknown` support upstream in late 2025, backed by the `sqlite-wasm-rs` ecosystem (which is mature enough that Diesel uses it). **Verify the minimum rusqlite version and the exact backend mechanism at spike time** — the support is young.

**Why a replica and not a query-the-server live mode.** Having the live console issue queries against the server's structured API would split the code path (file mode has no server), which is exactly the divergence this architecture exists to prevent. It would also make time-travel need server support. As a replica, time-travel works against a *live* agent the same way it works against a package: fold to seq N locally, diff two folds locally, zero server involvement.

**The fallback.** If the WASM spike fails (see stage 0), fall back to the pure-core carve-out: handlers dispatch on `(type, version)` and produce pure graph state or deltas, the SQLite layer applies them, and an equivalence test (fold the same log both ways, dump, compare) keeps the two paths byte-honest. More work, still sound, and the eval harness's predicate and brief scenarios run against materialized output, so the refactor has a net under it.

## The bridge interface

The spec already defines the seam: the `Store` is `append(events)`, `read_from(seq)`, `subscribe()`. The bridge is the **read half of that seam**, surfaced to the frontend:

```
EventSource
  catch_up(from_seq) -> Event[]     -- finite backlog
  tail() -> stream<Event>           -- live only; a package source just ends

PackageSource: one run's embedded Event[] from the eval package. No tail.
LiveSource:    GET /control/events?from=<seq> + an SSE or WebSocket tail. Later phase.
```

Both feed the same WASM materializer; the views query the resulting local graph and never know which source fed it. Writes are **not** part of the bridge — see the live-mode section.

## Stage 0: the spike (gating, do this first)

Half-day-shaped question with a yes/no answer; everything else is contingent on it.

1. Compile the materializer crate (or a minimal crate depending on it) to `wasm32-unknown-unknown` with rusqlite's wasm support.
2. In a browser (or wasm-pack test), feed it a real event log — extract one run's `Event[]` from any package in `eval/` — and run a handful of graph queries against the result.
3. Check that FTS5 is present in the bundled SQLite build (the official SQLite wasm build carries it; verify for this binding). FTS is not needed for the first-phase views, but knowing now changes nothing vs. discovering later.
4. Note the wasm binary size. Expect roughly 1–2 MB; for an operator tool loaded once, anything in that region is fine.

If this works, proceed. If it is flaky or blocked, take the fallback above and re-plan the wasm wrapper around the pure core.

## The WASM module's API shape

A `wasm-bindgen` wrapper over the materializer:

- **Input:** pass the package (or the event array) as raw bytes and parse with serde *inside* the module — one copy across the boundary, no JSON.parse-then-convert double handling. 5–6 MB packages are well within comfortable range.
- **Fold control:** `apply(events)` for incremental feeding (the live tail appends through the same path), and fold-to-seq-N for time-travel. Folding a package-sized log is milliseconds in native-ish code; re-folding from 0 for a time-travel scrub is acceptable, and caching folds at checkpoints is an optimization to defer.
- **Queries:** purpose-built methods per view (list memories by namespace/tag/recency, get one memory with entries/tags/links/history/`same_as` class, list conversations/sessions/turns, etc.), returning `serde-wasm-bindgen` values typed by the ts-rs bindings.
- **The console sees everything.** The console is operator-only and deliberately bypasses the visibility predicate and the live filters (superseded entries and soft-deleted memories render, marked as such). This is per spec — history surfaces are the point — and it means the visibility predicate does not need to be exposed through the wrapper at all for the first phase.
- **Re-derive deterministic projections rather than logging them.** A second, quieter win of compiling the real Rust in: any deterministic derived computation can be *re-run* by the wrapper instead of needing to ride in the log. The load-bearing case is the **brief trace** — the spec (§Observability) calls for "which memories were considered, which were filtered and why," but `memory::brief::compose` currently returns only the rendered string (`Result<String, BriefError>`) and no event carries the trace; the composer computes the negative space and discards it. Because composition is a pure function of the graph, the present set, and the clock (spec principle 6), and the replica holds all three (fold to the session's seq, present set from `SessionStarted.participants`, time from `started_at`), the wrapper can re-run a trace-returning `compose` variant and reconstruct *why a memory was or wasn't in the brief* — answerable without a `BriefTrace` event bloating every session. The same trick decomposes the system prompt: `assemble()` concatenates scaffold + identity-from-`self` + API reference + vocabulary + brief + time, and each constituent is either in the log (scaffold via `PromptTemplateRegistered`, brief via `SessionStarted`), re-derivable from the folded graph (`self` entries, vocabulary), or build-code compiled into the module (`render_api_reference`), so the console can annotate the assembled `ModelCalled.request.system` by provenance — which a TypeScript reimplementation never could. The one caveat is the regenerative-replay asterisk: re-derivation runs *current* composer logic, so it is exact for the eval viewer and a live-recent agent, and carries a "logic may have changed" note for cross-version time-travel. Adding the trace-returning `compose` variant is composer-local work (a return value, not an event), and is the cheapest form of the spec's "emit it from the start" intent.

## The type contract and artifact discipline

Rust stays the single source of truth, exported two ways:

- **Types:** the ts-rs bindings in `console/src/types` (the existing `ts` cargo feature; checked in, "generated — do not edit" headers). The entry type is `EvalPackage`; the embedded log is `Event[]` with the `EventPayload` union.
- **The materializer wasm bundle:** built from the `console-wasm` wrapper crate (`crates/console-wasm`) into `console/src/wasm` (`console_wasm.js` loader, `console_wasm_bg.wasm`, `.d.ts`), and **checked in** next to the types. A ~2 MB blob in git is mildly ugly, but it preserves the property that a fresh checkout (or a frontend-only dev) gets a working console with no Rust toolchain.

**One command regenerates both:** `./console/regen.sh` (re-execs into the nix-shell for the wasm C toolchain; runs the ts-rs export, then `cargo build --target wasm32-unknown-unknown` + `wasm-bindgen --target web` + `wasm-opt -Oz`). Rerun and commit whenever a wire type or the materializer changes. This uses `wasm-bindgen` + `wasm-opt` directly rather than `wasm-pack`, since the tools are already on `PATH` and in the dev shell, and it avoids `wasm-pack`'s separate `wasm-opt` download in a sandboxed build.

The Rust side changes in two bounded ways from the original "untouched" intent: the `console-wasm` wrapper crate and its build workflow, and the `zuihitsu-core` carve-out that makes the materializer wasm-compatible (done — see the git history). The materializer's *logic*, the harness, and the server are unchanged. (The live phase later adds the `/control` events endpoint and snapshot download — see below.)

## Views: what each consumes, and build order

Two kinds of consumption, both source-agnostic:

- **Straight off the event stream** (no graph needed): the **Events** view (filter by time, type, target, participant, source) and the **Conversation** view. Log-only telemetry — `ModelCalled`, `LuaExecuted` results, turns — renders directly from the events. The fiddliest specced piece lives here: reconstructing full prompts from the `ModelCalled` request deltas (walk the `(turn_id, phase)` group from its `Base`, concatenate, check the SHA-256 digest; render a digest mismatch loudly, never silently). Aborts and errors render distinctly from successful blocks.
- **Queries against the local graph**: the **State** view (browse by namespace/tag/recency; memory page with contents + `told_by` + visibility, tags, links, description, per-memory history, `same_as` class) and **Time-travel** (fold to seq N, render, diff two folds).
- **Eval-package chrome**, outside the log: the scenario/run overview (verdicts, judge rationales, pass rates, latency and token metrics from `ScenarioReport`/`RunRecord`), and a **trends** page over the tracked `eval/history.jsonl`.

Build order, chosen for payoff-per-effort while eval debugging is the live use case: **scenario overview → Conversation → Events → State → Time-travel**, trends whenever convenient. The scenario overview is cheap and immediately answers "which runs failed and why"; Conversation is the actual payoff ("what was the agent thinking," made literal); Events is a cheap filter UI; State and Time-travel ride on the wasm graph.

Virtualize long lists (events, turns) from the start; a run's log is thousands of rows and retrofitting virtualization is worse than starting with it.

## Live mode (later phase — design now, build later)

- **Catch-up reuses the snapshot machinery.** Streaming a years-old log from seq 0 on every console open is too slow. The server already takes graph snapshots (`VACUUM INTO`, tagged with the `graph_head` seq). Live catch-up: download the latest snapshot file, open it directly in the wasm SQLite (it opens databases from bytes), tail events from its `graph_head`. This is the boot path's `min(graph_head, latest_snapshot)` discipline applied over the wire — the console catches up the way the server itself boots. No new invention, just the recovery machinery reused.
- **Writes and authority stay server-side.** A `same_as` merge, a memory delete, the imprint interview — `/control` POSTs landing as `source: Operator`, authenticated per the loopback-trusted/remote-keyed model in the spec. The replica never mutates locally; it sees the resulting events arrive on the tail and re-renders. This keeps "no back-door state mutation" and the unbroken audit trail intact.
- **Vector search stays server-side** (the replica has no embedder). FTS over the local graph is available if the wasm build carries FTS5; plain substring/prefix search over the in-memory projection is an acceptable floor for an operator tool.
- **The Lua REPL** splits: the authoritative REPL (real authority, real locks) hits the server. A *read-only* REPL against the local wasm graph is a possible later nicety — the spec's console REPL is already "queries are synchronous, minus async I/O," which is exactly what a client-side fold can serve — but it is not a commitment.
- **Spec consequence to fold back in once confident:** the Access model's "read-only second SQLite connection" path is superseded by the replica-over-the-event-stream, which covers the on-box operator through the same interface as the remote one. Small spec edit, one less special case.

## Tech stack

- **Frontend:** React + Tailwind CSS + Vite + TypeScript. Current stable versions; nothing pins a minor.
- **Rust↔browser:** `wasm-bindgen` + `wasm-pack` for the wrapper crate, `serde-wasm-bindgen` at the boundary, raw-bytes-in/serde-inside for bulk input.
- **No backend to run for the first phase.** The viewer loads a package file (file picker and drag-drop; in dev, serving `eval/` statically so a `?file=` query param works is a nice touch).
- **Testing:** vitest for the TS layer (sources, delta-walk reconstruction, view models). The materializer itself is tested on the Rust side already; the wasm wrapper gets a smoke test in CI if cheap, otherwise the spike harness is kept runnable.

## Aesthetic

Japandi, per the spec — and it will quietly die if expressed as ad-hoc Tailwind utility classes. **Define the design tokens before the first component**: CSS variables wired into the Tailwind theme for the palette (warm off-white/oat ground, sumi-ink text, clay and sage as the only accents, used sparingly), a real modular type scale doing the structural work, hairline rules and negative space instead of boxes and shadows. The console is read for hours; calm and legible beats dense and chromed. The same tokens carry forward to every operator-facing surface.

## Getting data to develop against

Eval packages are large and gitignored (`/eval/*.json`); a corpus already exists in `eval/` (5–6 MB each, real qwen/gemma runs — `latest.json`, `qwen-full.json`, etc.). Generate a fresh one (needs a configured model endpoint — see `config.toml`):

```
cargo run -p zuihitsu-eval -- run --runs 1 --scenario tag_room_confidential --out eval/sample.json
```

The tracked trend file `eval/history.jsonl` (one deterministic line per run) is the source for the trends page.

## The harness CLI (your data producer — behavior unchanged)

The harness is the `crates/eval` crate (binary `eval`), a workspace member over the `zuihitsu` library.

- `eval run` — `--runs N` (default 8), `--concurrency N` (default 1; the local endpoint serializes inference), `--scenario <substrings>` (comma-separated OR-filter; no exclude flag), `--out <path>` (default `eval/latest.json`), `--config <path>` (default `config.toml`). Exits non-zero only on a gating regression; skips cleanly when no endpoint is configured.
- `eval export-types <dir>` — writes the TypeScript contract (above).

## Sequencing summary

0. **Spike**: materializer → wasm32-unknown-unknown with bundled SQLite; feed a real log, query it, check FTS5 and binary size. Gates everything.
1. **Wasm packaging**: the wrapper crate, the query API, the regenerate-and-commit workflow alongside the ts-rs types.
2. **Console first phase**: scaffold Vite/React/Tailwind with the design tokens; `EventSource` + `PackageSource`; views in the order above.
3. **Live phase** (with the live wiring, not before): `/control` events endpoint + snapshot download on the server, `LiveSource` + snapshot catch-up in the client. Nothing above the source changes.

## Known risks

- rusqlite's wasm support is roughly six months old; pin versions, expect rough edges, and keep the pure-core fallback in reserve.
- FTS5 presence in the wasm build is assumed-likely, unverified. First-phase views don't need it.
- The checked-in wasm artifact needs the same staleness discipline as the type bindings: regenerate both with one command, ideally with a CI check that they match the Rust source.
- The `ModelCalled` delta-walk reconstruction is well-specified but fiddly; budget it as real work, and treat a digest mismatch as a first-class rendered error.
- **MCP tool results are captured only as far as the agent surfaced them.** An `mcp.<server>.<tool>(...)` call returns a projected value into the Lua block, and `LuaExecuted.result` records the block's rendered final value plus printed output — so an MCP result is in the log exactly to the extent the agent returned, printed, or folded it into that value. This covers the debugger-relevant case ("what did the agent see from the fetch, and act on") by construction, since unsurfaced output never entered the agent's context either. The genuine residual: intermediate MCP output the agent consumed internally and discarded is not recorded, and the exact external content is not replay-faithful (regenerative replay re-fetches a world that may have moved — spec §Known limitations). Both are inherent to external I/O, not a console gap; the console should render what `result` carries and not imply it holds the full raw response.
