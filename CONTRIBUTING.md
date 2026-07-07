## General conventions

### Correctness over convenience

- Model the full error space—no shortcuts or simplified error handling.
- Handle all edge cases, including race conditions, signal timing, and platform differences.
- Use the type system to encode correctness constraints.
- Prefer compile-time guarantees over runtime checks where possible.

### User experience as a primary driver

- Provide structured, helpful error messages that can be rendered with an appropriate library at a later stage.
- Make progress reporting responsive and informative.
- Maintain consistency across platforms even when underlying OS capabilities differ. Use OS-native logic rather than trying to emulate Unix on Windows (or vice versa).
- Write user-facing messages in clear, present tense: "Frobnicator now supports..." not "Frobnicator now supported..."

### Pragmatic incrementalism

- "Not overly generic"—prefer specific, composable logic over abstract frameworks.
- Evolve the design incrementally rather than attempting perfect upfront architecture.

### Production-grade engineering

- Use type system extensively: newtypes, builder patterns, type states, lifetimes.
- Use message passing or the actor model to avoid data races in concurrent code.
- Test comprehensively, including edge cases, race conditions, and stress tests.
- Pay attention to what facilities already exist for testing, and aim to reuse them.
- Getting the details right is really important!

### Documentation

- Use inline comments to explain "why," not just "what".
- Don't add narrative comments in function bodies. Only add a comment if what you're doing is non-obvious or special in some way, or if something needs a deeper "why" explanation.
- Module-level documentation should explain purpose and responsibilities.
- **Always** use periods at the end of code comments.
- **Never** use title case in headings and titles. Always use sentence case.
- Always use the Oxford comma.
- Don't omit articles ("a", "an", "the"). Write "the file has a newer version" not "file has newer version".
- Comments describe the present state. Reserve past-tense narration for the rare case where history explains a standing "why".
- Prose that names a namespace concept links the canonical enum ([`Namespace::Person`]), never a literal prefix string. Concrete handles, prefix-string argument examples, and agent-facing text are syntax, not prose, and are exempt.
- Core code (`src/`, `crates/core`) never references the eval harness, scenarios, or run statistics in comments — state the behavior and its hazards directly. Only the eval crate may reference itself.

## Code style

### Rust edition and linting

- Use Rust 2024 edition.
- Ensure the following checks pass at the end of each complete task (you do not need to do this for intermediate steps):
  - `cargo +nightly fmt --all -- --check`
  - `cargo clippy --all --all-targets -- -D warnings`
  - `cargo test --workspace`

### Type system patterns

- **Builder patterns** for complex construction (e.g., `TestRunnerBuilder`)
- **Type states** encoded in generics when state transitions matter
- **Lifetimes** used extensively to avoid cloning (e.g., `TestInstance<'a>`)
- **Restricted visibility**: Use `pub(crate)` and `pub(super)` liberally
- **Parameter structs over long argument lists**: when a function approaches the `clippy::too_many_arguments` threshold, bundle the cohesive parameters into a struct (a request struct, or a shared seam like `Engine { store, graph, clock }` that several call shapes pass along) rather than threading more positional arguments. **Never** silence the lint with `#[allow(clippy::too_many_arguments)]`; the lint firing means a struct is wanted. Recognized closed sets of values (relation labels, tags) likewise ride as enums, not bare strings.

### Error handling

- Do not use `thiserror`. Instead, manually implement `std::fmt::Error` for a given error `struct` or `enum`.
- Group errors by category with an `ErrorKind` enum when appropriate.
- Provide rich error context using structured error types.
- Two-tier error model:
  - `ExpectedError`: User/external errors with semantic exit codes.
  - Internal errors: Programming errors that may panic or use internal error types.
- Every error's `Display` leads with a `<context>:` prefix naming the subsystem or operation it belongs to (e.g. `event store: …`, `lua: block commit failed: …`, `could not open the event log at /path: …`), then the cause. Aggregating errors prefix their own layer's context and delegate the inner error, so a chained error reads as nested context (`turn: lua: block commit failed: event store: …`). Add resource context like a path at the layer that has it. Avoid bare "failed to {x}" glue.

### Async patterns

- Do not introduce async to a project without async.
- Use `tokio` for async runtime (multi-threaded).
- Use async for I/O and concurrency, keep other code synchronous.
- Use `parking_lot::Mutex` for synchronous locks (the default); its guard is non-poisoning and must never be held across an `.await`. Reserve `tokio::sync::Mutex` for the rare guard that must survive an `.await`, since most locks are acquired, used, and dropped within a synchronous span.

### Logging

- Use `tracing` for diagnostic and operational logging throughout, emitting at meaningful points, not noisily.
- Install the subscriber only in binaries, and send logs to stderr.
- The CLI is an operator/diagnostic tool, so its output goes through `tracing` too — the user-facing interface is the web frontend. Reserve `stdout`/`println!` for genuine machine-readable command output if a command ever needs it.

### Module organization

- Use `mod.rs` files to re-export public items.
- Keep module boundaries strict with restricted visibility.
- Use `#[cfg(unix)]` and `#[cfg(windows)]` for conditional compilation.
- **Always** import types or functions at the very top of the module, with the one exception being `cfg()`-gated functions. Never import types or modules within function contexts, other than this `cfg()`-gated exception.
- It is okay to import enum variants for pattern matching, though.
- When a path is used more than once in a module, import it at the top of the module (the specific items, not the module) rather than repeating the fully-qualified path at each call site. A path used only once may stay fully-qualified.

Within each module, organize code as follows:
1. **Public API first** - all `pub` structs, enums, and functions at the top
2. **Private implementation below** - constants, helper functions, and internal types
3. **Order by use** - private items should appear in the order they're called/used by the public API (topological order)

### Memory and performance

- Use `Arc` or borrows for shared immutable data.
- Use `smol_str` for efficient small string storage.
- Careful attention to cloning referencing. Avoid cloning if code has a natural tree structure.
- Stream data (e.g. iterators) where possible rather than buffering.

### Database access (SQLite)

- Run every `query_map` and multi-column `query_row` through the shared `db::query_map_into` / `query_opt_into` helpers, passing a mapping closure. They own the prepare-iterate-collect plumbing and are generic over the error type, so a mapper that decodes a row and then does serde/ULID work `?`-chains into the layer's own error rather than hand-rolling a closure that returns a tuple plus a second loop that converts it.
- Each error type that flows through the helpers implements `From<rusqlite::Error>` (so the helper's and the mapper's `?` convert backend failures). That `From` is the conversion path; reserve a `map_err` shim for the few reads that stay on a bare `query_row`.
- Decode a row's columns with rusqlite's tuple `TryFrom` — `let (seq, recorded_at, payload): (i64, i64, String) = row.try_into()?;` — **only when the unpack stays a single line** (roughly three or four narrow columns). For wider rows, fall back to explicit per-column `row.get("column")?` **by name**: a multi-line tuple-of-types buys nothing over named gets and reads worse, and naming the columns is order-safe where counting positions is not. Reserve positional `row.get(0)` for a lone scalar, where neither a tuple nor a name pays off.
- Keep the row-decoding mapper **beside its query**, as a local closure (or a small free fn for a genuinely shared shape). Do not hoist decoding into a per-type `TryFrom<&rusqlite::Row>` impl: a single impl presumes every query reads that type identically, which the schema does not guarantee.

### Reaching through smart pointers

- To borrow the value inside a lock guard, a `Box`, or an `Arc`, prefer `.as_ref()` / `.as_mut()` over a manual double-deref: write `engine.store.lock().as_ref()` and `engine.store.lock().as_mut()`, not `&**engine.store.lock()` and `&mut **engine.store.lock()`. The named form reads as "borrow the store" rather than as deref bookkeeping, and it is the form already used throughout (`Settings::from_store(store.lock().as_ref())`, `genesis::rollout(store.lock().as_mut(), …)`). The same applies to an `Arc<dyn Trait>`: `model.as_ref()`, not `&**model`.

### Instance features

An `InstanceFeatures` bitfield (`crates/core/src/instance_features.rs`) controls which Lua API features an instance enables — `linking`, `tagging`, `merging`, `calendar`, plus always-on `memory` and `context`. A disabled feature is dropped from **three gates in lockstep**:

1. **Lua registration** (`tables.rs`): the feature's methods and module tables are not installed, so calling them yields the standard Lua "attempt to call a nil value" error — a teachable failure, not a silent no-op.
2. **API reference** (`reference.rs`): `api_reference(&features)` omits the feature's entries, so the system prompt's "What you can do" section does not describe them.
3. **Scaffold dotpoints** (`genesis.rs`): `default_templates(&features)` drops the dotpoints that teach the practice, so the prompt does not teach the agent to use something it cannot.

The scaffold is **baked into the event log at genesis** and read back verbatim at turn time, so feature-gating it is a genesis-time decision: the features set at `genesis::rollout` decide which dotpoints persist, and a later turn reads exactly that. The Lua registration and API reference, by contrast, read the running binary's `InstanceFeatures` fresh each turn. This is fine for the eval use case (each run is a fresh instance born with the scenario's features) and for a live deployment (features are set at construction, before genesis).

**When adding a new Lua API function**, ensure it fits under an existing feature group, or add a new feature flag and wire all three gates. A function installed but not documented is undiscoverable; one documented but not installed is a confusing error. The three-gate invariant is aspirational, not enforced by the compiler — code review is the guard against drift. Features are coarse-grained (a practice + its functions), not per-function, because the scaffold's dotpoints teach practices that span several calls.

### The Luau sandbox

The block VM is Luau, frozen with `Lua::sandbox(true)` (`src/agent/lua/mod.rs`). Only the pure standard libraries load; `os`, `io`, `package`, `require`, `debug`, and the code-loading globals are absent, so a block stays deterministic under replay and cannot reach the host. Metamethods on our own objects and our own installed functions are fair game — shape the API however reads best. Standard-library semantics, by contrast, are never changed: a semantics-preserving error-rewrite shell (the lenient `table.concat`, installed before the freeze) is the ceiling — it delegates the real call untouched and only rewords the failure. Agent-facing surfaces name the language "Luau", and teach text assembly through interpolation (a backtick string stringifies a handle), not through library gymnastics.

### The connector contract

Console and platform URLs never reach the agent. A connector normalizes every deep link to a canonical `[turn:<id>]` token before a message posts (`normalize` in `crates/core/src/turn_ref.rs`); the console composer is one such connector. Agent-facing surfaces — the scaffold, the API reference, error messages — speak tokens only, never a URL.

### The seed ontology

The relations seeded at genesis (`seed_relations()` in `src/agent/genesis/seed.rs`) are a minimum-viable ontology: the structural universals the system itself leans on — identity, participation, composition, origin, operatorship, and acquaintance. Social and environmental semantics are the agent's to coin at runtime, not ours to preload. Document any change to the set at `seed_relations()`, and point new prose there rather than restating it.

### Prompt-surface discipline

The scaffold teaches load-bearing practices as principles, stated once. API options and mechanics live in the reference, stated once — the scaffold does not restate them. Teachable errors are the syllabus for rare mistakes: a slip the agent makes seldom is caught and reworded at its point of failure rather than pre-taught in the prompt. Changing a template body bumps its `(name, version)` pair (`TemplateDef`, `src/agent/genesis/mod.rs`), so an older `produced_by` still names the body it was generated under and the genesis manifest hash moves.

## Testing 

### Testing tools

- **test-case**: For parameterized tests.
- **proptest**: For property-based testing.
- **insta**: For snapshot testing.
- **libtest-mimic**: For custom test harnesses.
- **pretty_assertions**: For better assertion output.

### Testing conventions

- Tests use the in-memory backends by default (`MemoryStore`, `Graph::open_in_memory`, `SqliteVectorIndex::open_in_memory`): the system is a pure function of the event log modulo declared nondeterminism, so replay through memory exercises exactly that function. Reach for a disk-backed backend only in a test that guards a genuine filesystem property, and name it for that.
- Do not write a test that only exercises serde or a derive. A round-trip earns its place only when it guards a real wire: a versioned payload, the control API, or a package file.
- No personal names in fixtures.
- Scenario dates are computed from `RUN_START_MS` and the shared time constants (`MILLIS_PER_DAY`, …), never bare epoch literals.

## Evaluations

The eval harness in `crates/eval` runs the agent through a suite of behavioural scenarios against a local model, judges each run against per-scenario oracles, and writes a package (`eval/<name>.json`) that the console renders and the `analyze` subcommand reads. A run drives a local inference server, so it needs a configured model (`config.toml`) and a GPU, and it is kept out of `cargo test`.

When you change the agent's behaviour — a prompt, a tool, a planning step, anything the agent does — write an eval that captures that behaviour so a later change can be assessed against it. The new scenario is the regression test for the edit you just made: if a future tweak regresses it, the eval run goes red.

### Running an eval

```
cargo run -p zuihitsu-eval --bin eval -- run --name <name> --config config.toml
```

- The run writes `eval/<name>.json` and a resumable `.jsonl` sidecar. `eval/` is gitignored apart from the tracked `history.jsonl` trend record, so name runs descriptively.
- `--runs N` sets the runs per scenario (the statistical N; default 8). `--scenario a,b,c` keeps only scenarios whose names contain one of the comma-separated substrings.
- Serving is on by default — the live console is at http://127.0.0.1:7878/ while the run proceeds, and the process exits when it finishes. Keep it on: watching the deliberation unfold live is the point of running an eval, not just the pass/fail at the end. `--no-serve` is for CI or headless machines only. Do not pass `--serve-after-completion` unless explicitly requested: it leaves the process holding the console port indefinitely, which silently blocks the next run's live view, and a finished run is reviewed by loading `eval/<name>.json` into the viewer anyway. If a serving process does linger, stop it before the next launch; `--resume` recovers a run that had to be bounced.
- The exit code is the gating signal: success when every gating oracle held, failure (logging `a gating safety oracle regressed`) when one slipped. A gating bar (`Bar::gating()`, or `Bar::gating_at(rate)` for a tolerance) fails the harness when the held rate of its gating verdicts falls below `min_rate`. `Bar::holds` decides it: at the default `min_rate` of 1.0 it reads the pass/fail boolean directly — the one-slip discipline for must-not-surface safety properties — while a `min_rate` below 1.0 compares the held rate and suits model-judgment behaviors with a known error band. A metric bar is a should-surface rate, reported against a threshold but never failing the run.

### Analyzing an eval

```
eval analyze eval/<name>.json                      # per-scenario summary (rate, bar, gating)
eval analyze eval/<name>.json -b eval/<base>.json  # ... with the Δ, regressions, and improvements vs a baseline
eval analyze eval/<name>.json -f -s <scenario> -e <events>     # dump the failed runs' complete deliberation traces
```

Reach for the `--failures`/`-f` dump when deciding the next prompt or code edit. It starts with a cross-scenario rollup of every missed verdict, grouping by criterion so a single behavioural thread is visible across scenarios. For each failed run it prints the missed oracles with their rationale, then the whole deliberation: the agent's reasoning, the Lua it ran, and what came back. `--scenario`/`-s` filters by name substring; `--events`/`-e` adds a compact summary of the events whose payload type name contains the substring (for example, `MemoryContentAppended`, `ScheduledJobFired`, or `EntryTemporalResolved`); `--limit` caps the runs shown per scenario; and `--truncate N` clips long reasoning and scripts (`0` keeps them whole).

### Designing a scenario

- An oracle must align with the system's own rules. A gate must not punish behavior the visibility model permits — if the property under test is privacy, encode the isolated fact as a confidence, not a gating failure.
- Prefer a structural check for an exact property; reserve the judge for a genuine language judgment. A list of needle phrases is a smell — it pins wording, not behavior.
- When a new scenario overlaps an existing one, distinct tested properties justify it. Trend history is a reason not to mutate an existing scenario's criteria set: add a scenario rather than redefine an old one out from under its history.

## Frontend (the console)

The web console lives in `console/` — Vite, React, TypeScript, and Tailwind CSS v4, with the React Compiler enabled (so the data-heavy views auto-memoize; prefer plain derivation over hand-written `useMemo`/`useCallback`). See `console/PLAN.md` for the architecture and `docs/spec.md` → **Observability** for what the views are.

### Checks

Run these from `console/`; all four must pass, and CI enforces them:

- `npm run typecheck` — `tsc -b` with no errors.
- `npm run lint` — ESLint, including the `react-hooks` and React Compiler rules (the views must stay within the Rules of React the compiler relies on).
- `npm run format:check` — Prettier. `npm run format` writes the fixes.
- `npm run build` — the production build.

### Conventions

- **Rust is the source of truth for the wire contract.** The TypeScript bindings in `console/src/types/`, the settings metadata in `console/src/types/settings-metadata.ts`, and the wasm materializer bundle in `console/src/wasm/` are *generated* — never hand-edit them, and never commit them (both directories are gitignored). Regenerate all three with `./console/regen.sh` (direnv provides the nix-shell locally; CI provides its own toolchain) whenever a wire type, the materializer's logic, or a settings `///` doc comment changes. This is not just for schema changes — any edit to the graph queries, the projection code, or anything the wasm exposes must be followed by a rebuild, or the console runs against a stale materializer. The CI `console` job runs `./console/regen.sh` before its checks, so it always builds against the current Rust source — a PR that changes a wire type, the materializer, or a settings doc comment is validated directly, with no separate commit step. Linting and formatting skip these directories.
- **Cast wasm crossings in one place.** The `Replica` wrapper (`src/lib/replica.ts`) is the only place the wasm `JsValue` results are typed; views consume the typed surface.
- **The aesthetic is Japandi**, expressed as design tokens in `src/app.css` (`@theme`): a warm paper ground, sumi-ink text, clay and sage accents used sparingly, a real type scale, and hairline rules. Reach for the tokens (`text-ink`, `border-line`, `font-serif`, `text-clay`, …) rather than ad-hoc colors, so the system stays coherent. Craft over chrome — calm and legible over dense.
