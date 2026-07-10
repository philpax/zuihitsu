# zuihitsu

> **Provisional.** This README is an early entrypoint to establish the project's terminology; it will be rewritten at a later stage.

zuihitsu is an agent system: the software a single conversational agent runs on. It takes a language model — which on its own only maps a prompt to a reply, forgetting everything between calls — and gives it a memory, a continuous identity, and the means to converse and act. The agent meets people across platforms, remembers what each has told it, keeps confidences between them, and surfaces what is relevant when a conversation begins.

zuihitsu is not the agent. One instance hosts exactly one agent, but the agent is the persona that lives on top — named and shaped by whoever runs the instance. zuihitsu is everything beneath it: the event log, the memory that log folds into, and the loop that drives the agent's turns. (Throughout, "zuihitsu" is written lowercase.)

## How it works

The agent's entire life is a single **event log**, read from the beginning. The log is append-only and the only source of truth: every memory, conversation, link, and decision is an event, and nothing is ever silently overwritten — a correction is a new event that supersedes the old one. Wipe everything else and the agent rebuilds from the log alone.

Folding that log forward produces the **memory graph** (and a vector index for search): the materialized state the agent reads and the console renders. It is derived, disposable, and reconstructible from the log at any point — which is what makes the whole history replayable and auditable.

The agent acts by reading and writing that memory. Each turn, it emits Lua against a small memory API — creating memories, recording facts, linking people, scheduling reminders — and zuihitsu commits the results as new events. Reaching the outside world happens through operator-configured MCP servers.

## Terminology

- **agent** — the persona zuihitsu hosts: unnamed by the system, named by the operator at creation. One per instance.
- **operator** — whoever runs the instance and holds the console. Trusted: creates the agent, inspects its state, and is the only authority that may edit the agent's `self`. In ordinary conversation the operator is just another participant.
- **participant** — anyone the agent talks to across a platform. Participants are not trusted with each other's confidences: the agent keeps an aside about one person from reaching them.
- **the agent server** — the one process that owns the log, the graph, and the model. Running `zuihitsu` with no subcommand boots it. Every other tool is a client of its API, and authority comes from the client's role, not from where it runs.
- **the console** — the web frontend: a read replica that folds the log through the same materializer the server runs (compiled to WebAssembly), so it shows the agent's exact state, scrubbable back through time. It is also where the operator converses with the agent.
- **session** — a bounded window of activity within a conversation. The contextual brief, which the agent is told at the top of its prompt, is frozen when a session opens, so a conversation keeps a stable footing.
- **self** — the agent's own memory of who it is, seeded at creation and writable only by the operator.
- **genesis** — creating an agent: rolling out the first events of its log, including the imprint interview in which it meets its operator.

## The pieces

- `zuihitsu` (no subcommand) — the long-running agent server.
- `zuihitsu <subcommand>` — the operator CLI, a client of the running server (`zuihitsu events`, `zuihitsu memory`, and so on).
- `console/` — the web console (Vite, React, and TypeScript). See `console/CONTRIBUTING.md`.
- `crates/eval/` — the evaluation harness: scenario-based behavioural tests run against a real model.

## Going deeper

- `docs/overview.md` — the design spec: the goals, the architectural principles, and the map of the per-area documents covering every subsystem in detail.
- `CONTRIBUTING.md` — the conventions and the checks each change must pass.
- `console/CONTRIBUTING.md` — an onboarding guide to the console's structure and motifs.
