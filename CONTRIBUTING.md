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

## Code style

### Rust edition and linting

- Use Rust 2024 edition.
- Ensure the following checks pass at the end of each complete task (you do not need to do this for intermediate steps):
  - `cargo +nightly fmt --all -- --check`
  - `cargo clippy --all-targets -- -D warnings`
  - `cargo test --workspace`

### Type system patterns

- **Builder patterns** for complex construction (e.g., `TestRunnerBuilder`)
- **Type states** encoded in generics when state transitions matter
- **Lifetimes** used extensively to avoid cloning (e.g., `TestInstance<'a>`)
- **Restricted visibility**: Use `pub(crate)` and `pub(super)` liberally
- **Parameter structs over long argument lists**: when a function approaches the
  `clippy::too_many_arguments` threshold, bundle the cohesive parameters into a struct (a request
  struct, or a shared seam like `Engine { store, graph, clock }` that several call shapes pass
  along) rather than threading more positional arguments. **Never** silence the lint with
  `#[allow(clippy::too_many_arguments)]`; the lint firing means a struct is wanted. Recognized
  closed sets of values (relation labels, tags) likewise ride as enums, not bare strings.

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

- Run every `query_map` and multi-column `query_row` through the shared `db::query_map_into` /
  `query_opt_into` helpers, passing a mapping closure. They own the prepare-iterate-collect plumbing
  and are generic over the error type, so a mapper that decodes a row and then does serde/ULID work
  `?`-chains into the layer's own error rather than hand-rolling a closure that returns a tuple plus a
  second loop that converts it.
- Each error type that flows through the helpers implements `From<rusqlite::Error>` (so the helper's
  and the mapper's `?` convert backend failures). That `From` is the conversion path; reserve a
  `map_err` shim for the few reads that stay on a bare `query_row`.
- Decode a row's columns with rusqlite's tuple `TryFrom` — `let (seq, recorded_at, payload): (i64,
  i64, String) = row.try_into()?;` — **only when the unpack stays a single line** (roughly three or
  four narrow columns). For wider rows, fall back to explicit per-column `row.get("column")?` **by
  name**: a multi-line tuple-of-types buys nothing over named gets and reads worse, and naming the
  columns is order-safe where counting positions is not. Reserve positional `row.get(0)` for a lone
  scalar, where neither a tuple nor a name pays off.
- Keep the row-decoding mapper **beside its query**, as a local closure (or a small free fn for a
  genuinely shared shape). Do not hoist decoding into a per-type `TryFrom<&rusqlite::Row>` impl: a
  single impl presumes every query reads that type identically, which the schema does not guarantee.

### Reaching through smart pointers

- To borrow the value inside a lock guard, a `Box`, or an `Arc`, prefer `.as_ref()` / `.as_mut()`
  over a manual double-deref: write `engine.store.lock().as_ref()` and
  `engine.store.lock().as_mut()`, not `&**engine.store.lock()` and `&mut **engine.store.lock()`. The
  named form reads as "borrow the store" rather than as deref bookkeeping, and it is the form already
  used throughout (`Settings::from_store(store.lock().as_ref())`, `genesis::rollout(store.lock()
  .as_mut(), …)`). The same applies to an `Arc<dyn Trait>`: `model.as_ref()`, not `&**model`.

## Testing 

### Testing tools

- **test-case**: For parameterized tests.
- **proptest**: For property-based testing.
- **insta**: For snapshot testing.
- **libtest-mimic**: For custom test harnesses.
- **pretty_assertions**: For better assertion output.
