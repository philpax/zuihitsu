# The console

The console is the operator's web interface to the agent: a place to read what the agent
remembered, watch it think, and drive control actions. Its first and still-primary job is the
eval-package viewer, but the same code also serves a live agent and a standalone landing page. This
guide orients you in the shape of the thing; the checked conventions and required checks live in the
root `CONTRIBUTING.md` → **Frontend (the console)**. Read that for the rules — this document is the
map.

## What it is: a materialising read replica

The console never asks a server what the state is. It obtains the event log from a *source*, folds
it through the agent's own materialiser compiled to wasm — real Rust, real SQLite bundled into the
module — and renders every graph view as a query against that local fold. A source is anything that
delivers the log: a stored run's embedded `Event[]` today, a live agent's `/control` stream tomorrow.
Everything above the source is shared by construction.

This shape was chosen for faithfulness. A TypeScript re-fold of the events would show a *second
opinion* about what the events imply, and every divergence from the agent's real projection would be
a lie in a debugger whose whole purpose is to answer "what was the agent thinking." Compiling the
real materialiser in means the console renders the exact fold the agent lives in — it can never
disagree with the agent's own projection, and a new event type or payload version is handled once, in
Rust, not twice forever.

The cost is that the console folds the whole log. That is milliseconds for an eval package and fine
for an operator tool loaded once, but live-scale catch-up (snapshot download, windowing, incremental
tailing) is deferred. Time-travel re-folds from zero
rather than caching checkpoints, which is an optimisation left for when it hurts.

## How it is structured

**Three serving modes, one bundle.** The serving binary announces its mode at runtime via
`window.__APP_MODE__` (a template token replaced at serve time), so one built bundle serves every
host (`src/App.tsx`):

- **`eval`** — the eval binary serves the full console pointed at its own same-origin live eval run.
- **`agent`** — the agent serves its own focused live view, rooted with no landing to return to.
- **`console`** (the fallback, token unreplaced) — the standalone build: landing page, package
  picker, and the trends screen over `eval/history.jsonl`.

**Directory motifs** (`src/`):

- `frames/` — the application shells, one folder per routed mode (`eval/`, `live/`, `landing/`,
  `trends/`), plus each shell's exclusive sub-components.
- `views/` — the routed timeline views shared across shells (conversation, events, state,
  time-travel diff, scenario overview, and so on).
- `components/` — shared-only: a component used by a single view or frame lives with that view or
  frame, not here.
- `lib/` — grouped strictly by concern: `api/` (server communication), `replica/` (the wasm bridge),
  `model/` (data shapes and derived state), `format/` (formatting), `nav/` (routing), and `view/`
  (view-context helpers that consume both model and replica).

**The Replica wrapper is the single typed wasm crossing.** `src/lib/replica/replica.ts` is the only
place the boundary is given types: composed query DTOs cross already typed (their declarations are
generated from the Rust structs), and the remaining core-view-type results are cast there and
nowhere else. Every view consumes that typed surface, never the raw boundary. The wrapper's Rust
half lives in `crates/console-wasm` over `zuihitsu-core`'s materialiser.

**Generated artefacts, never hand-edited.** Three trees are generated from Rust and gitignored: the
TypeScript bindings in `packages/wire/types/`, the settings metadata in
`packages/wire/types/settings-metadata.ts`, and the wasm bundle in `packages/wire/wasm/`.
`cargo build -p zuihitsu` regenerates all three automatically via the `console` Cargo feature (on by
default) — run it whenever a wire type, the materialiser's logic, the graph queries, or a settings
doc comment changes, or the console runs against a stale materialiser. CI's `console` job runs
`cargo build -p zuihitsu` before it checks, so a PR is validated against current Rust.

## The motifs and disciplines

- **Record facts, derive judgements.** Deterministic derived computations — the brief trace ("which
  memories were considered, which were filtered and why"), the system-prompt provenance breakdown —
  are re-run by the wasm wrapper against the folded graph rather than logged, because composition is a
  pure function of the graph, the present set, and the clock, and the replica holds all three. The
  wrapper carries a "logic may have changed" caveat for cross-version time-travel; it is exact for the
  eval viewer and a live-recent agent. Derive in the wasm and `model/` layers; do not re-implement the
  agent's logic and trust it not to drift. The one thing the log genuinely does not hold is an MCP
  result the agent consumed without surfacing it into a block's `result` — render what `result`
  carries, and do not imply it holds the full raw response.
- **Rust is the source of truth for the wire contract.** See the generated-artefact discipline above.
- **Typed is not visible.** Regenerating bindings makes a new field *typed*, not *rendered*. When a
  backend change adds or restructures state the console shows, the rendering components
  (`renderInteraction.tsx`, `renderPayload.tsx`, the brief components, …) must be updated to display
  it, following the existing patterns. A field that arrives typed but unrendered is a gap.
- **Plain derivation over memo hooks.** The React Compiler is enabled, so the data-heavy views
  auto-memoise; prefer deriving values inline over hand-written `useMemo`/`useCallback`, and stay
  within the Rules of React the compiler relies on.
- **Japandi via tokens.** The aesthetic is a warm paper ground, sumi-ink text, clay and sage accents
  used sparingly, a real type scale, and hairline rules — expressed as design tokens in `src/app.css`
  (`@theme`). Reach for the tokens (`text-ink`, `border-line`, `font-serif`, `text-clay`, …) rather
  than ad-hoc colours. The console is read for hours: calm and legible over dense and chromed.

## Checks

Run `npm run typecheck`, `npm run lint`, `npm run format:check`, and `npm run build` from `console/`;
all four must pass and CI enforces them. The root `CONTRIBUTING.md` → **Frontend (the console)** has
the details and the file-organisation rules.
