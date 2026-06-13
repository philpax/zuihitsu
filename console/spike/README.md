# Stage 0 spike — rusqlite + SQLite under wasm32-unknown-unknown

The console architecture (see `console/PLAN.md`) is a materializing read replica that folds the
event log through the agent's own rusqlite-backed materializer compiled to WASM. That only works if
rusqlite can reach SQLite under `wasm32-unknown-unknown`. This throwaway crate answers that.

## Result: green ✓

- **The auto-swap works with no patching.** `rusqlite 0.40` + `libsqlite3-sys 0.38` carry an
  `ffi-sqlite-wasm-rs` feature (on by default) that, on a wasm target, swaps the C-amalgamation
  build for [`sqlite-wasm-rs`](https://crates.io/crates/sqlite-wasm-rs), which compiles SQLite's C
  to wasm. No `[patch]`, no `libsqlite3-sys` surgery — `features = ["bundled"]` just works.
- **FTS5 is in the build.** The sqlite-wasm-rs compile carries `-DSQLITE_ENABLE_FTS5` (plus RTREE,
  math functions, and column metadata), so the live-search surface is available later if wanted.
- **It runs, not just compiles.** Driven from Node, an in-memory connection reports SQLite 3.53.0,
  creates a table, inserts, and reads the rows back across the wasm-bindgen boundary (via
  `serde-wasm-bindgen`). See `run.mjs`.
- **Size:** ~2.1 MB unoptimized `.wasm` (pre-`wasm-opt`), in the plan's expected range for an
  operator tool loaded once.

## The one environment gotcha (fixed by `shell.nix`)

Under Nix, the default clang *wrapper* injects host glibc include paths, which a freestanding wasm
compile must never see — it dies on `__GLIBC_USE` being undefined. The repo-root `shell.nix` points
`cc-rs` at an unwrapped clang for the wasm target only (`CC_wasm32_unknown_unknown`), leaving every
host C build (mlua, rusqlite's bundled SQLite, sqlite-vec) on the standard wrapped compiler.

## Reproduce

```
rustup target add wasm32-unknown-unknown        # once
nix-shell --run 'cargo build -p wasm-sqlite-spike --target wasm32-unknown-unknown --release'
wasm-bindgen --target nodejs --out-dir console/spike/pkg \
  target/wasm32-unknown-unknown/release/wasm_sqlite_spike.wasm
node console/spike/run.mjs
```

`wasm-bindgen` (the crate) is pinned to match the `wasm-bindgen` CLI on `PATH` (0.2.117); the two
must agree exactly. The real `console-wasm` wrapper crate should pin both to a nix-shell-provided
CLI so the pairing is reproducible.

## Next

The materializer is not separable as-is: `src/graph/` shares the `zuihitsu` lib crate with `mlua`,
`tokio`, `reqwest`, `axum`, and `async-openai`, none of which build for wasm. Its own in-crate
surface is lean (`event`, `ids`, `store`, `time`, `vocabulary`, `db`), so the next step is the
`zuihitsu-core` carve-out described in `console/PLAN.md` — extract the materializer and its closure
into a wasm-compatible crate, then build the real `console-wasm` wrapper around it. This spike crate
is deleted once that lands.
