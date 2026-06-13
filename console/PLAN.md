# The console — build plan

The **console** is the operator's web interface to the agent: state inspection, control actions, and — its first incarnation — the **eval-package viewer**. This is the build-level companion to the design spec, and it deliberately does not restate the spec. Read the spec for *what* to build and *why*; read this for the stack and the mechanics.

- **What and why:** `docs/spec.md` → **Observability** (the four views, the access model, the phases, the Japandi aesthetic) and **Validation and the eval harness** (what an eval package is, the judge, gating vs. metric).
- This directory (`console/`) is the frontend's home. Nothing is scaffolded yet — creating the project (`package.json`, Vite config, Tailwind) is the first task.

## Tech stack

- **Frontend:** React + Tailwind CSS + Vite + TypeScript. Use current stable versions; nothing here pins a minor.
- **No backend to run for the first phase.** The viewer loads an eval-package file. The agent server (Rust, Axum over loopback) is only needed for the later live-probing phase.

## The type contract (Rust is the source of truth)

The wire types are defined in Rust and exported to TypeScript via `ts-rs`; the frontend binds to the generated bindings rather than hand-written interfaces, so it can never drift from what the server emits.

- Regenerate (and re-run whenever a Rust wire type changes):
  ```
  cargo run -p zuihitsu-eval -- export-types console/src/types
  ```
- Output lands in `console/src/types/`. These are checked in — a fresh checkout (or a frontend-only dev) gets the contract without a Rust build — so regenerate and commit them whenever a Rust wire type changes. Each file carries a "generated — do not edit" header. The entry type is `EvalPackage`; the event log it embeds is `Event[]` with the `EventPayload` union.
- The export is gated behind the `ts` cargo feature, which the `zuihitsu-eval` crate enables; the main `zuihitsu` binary never carries `ts-rs`.

The one load-bearing fact about the shape: each run record embeds **the run's actual event log**, so every view is a reconstruction from `Event[]` — the same input a live agent's log will provide later. Factor the events→views logic as a pure layer independent of whether the log came from a file or a socket.

## Getting data to develop against

Eval packages are large and gitignored (`/eval/*.json`). Generate one (needs a configured model endpoint — see `config.toml`):
```
cargo run -p zuihitsu-eval -- run --runs 1 --scenario tag_room_confidential --out eval/sample.json
```
The tracked trend file `eval/history.jsonl` (one deterministic line per run) is the source for trend charts.

## The harness CLI (your data producer — you won't modify it)

The harness is the `crates/eval` crate (binary `eval`), a workspace member over the `zuihitsu` library.

- `eval run` — `--runs N` (default 8), `--concurrency N` (default 1; the local endpoint serializes inference), `--scenario <substrings>` (comma-separated OR-filter; no exclude flag), `--out <path>` (default `eval/latest.json`), `--config <path>` (default `config.toml`). Exits non-zero only on a gating regression; skips cleanly when no endpoint is configured.
- `eval export-types <dir>` — writes the TypeScript contract (above).
