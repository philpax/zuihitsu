# zuihitsu

> **Provisional.** This README establishes the project's terminology; a fuller rewrite comes later.

zuihitsu is an agent system: the software a single conversational agent runs on. A language model, on its own, maps a prompt to a reply and forgets everything between calls. zuihitsu gives it a memory, a continuous identity, and the means to converse and act. The agent meets people across platforms, remembers what each has told it, keeps their confidences separate, and surfaces what matters when a conversation begins.

zuihitsu is not the agent. One instance hosts exactly one agent, but the agent is the persona that lives on top, named and shaped by whoever runs the instance. zuihitsu is everything beneath it: the event log, the memory that log folds into, and the loop that drives the agent's turns. (Throughout, "zuihitsu" is written lowercase.)

## How it works

The agent's entire life is a single **event log**, read from the beginning. The log is append-only and the only source of truth: every memory, conversation, link, and decision is an event, and nothing is ever silently overwritten. A correction is a new event that supersedes the old one. Wipe everything else and the agent rebuilds from the log alone.

Folding that log forward produces the **memory graph**, plus a vector index for search: the materialised state the agent reads and the console renders. It's derived, disposable, and rebuildable from the log at any point, which is what makes the whole history replayable and auditable.

The agent acts by reading and writing that memory. Each turn, it emits Lua against a small memory API (creating memories, recording facts, linking people, scheduling reminders), and zuihitsu commits the results as new events. It reaches the outside world through operator-configured MCP servers.

## Terminology

- **agent**: the persona zuihitsu hosts, unnamed by the system, named by the operator at creation. One per instance.
- **operator**: whoever runs the instance and holds the console. Trusted: creates the agent, inspects its state, and is the only authority that may edit the agent's `self`. In ordinary conversation the operator is just another participant.
- **participant**: anyone the agent talks to across a platform. Participants aren't trusted with each other's confidences: the agent keeps an aside about one person from reaching another.
- **the agent server**: the one process that owns the log, the graph, and the model. Running `zuihitsu` with no subcommand boots it. Every other tool is a client of its API, and authority comes from the client's role, not from where it runs.
- **the console**: the web frontend, a read replica that folds the log through the same materializer the server runs (compiled to WebAssembly), so it shows the agent's exact state, scrubbable back through time. It's also where the operator converses with the agent.
- **session**: a bounded window of activity within a conversation. The contextual brief the agent reads at the top of its prompt is frozen when a session opens, so a conversation keeps a stable footing.
- **self**: the agent's own memory of who it is, seeded at creation and writable only by the operator.
- **genesis**: creating an agent by rolling out the first events of its log, including the imprint interview in which it meets its operator.

## The pieces

- `zuihitsu` (no subcommand): the long-running agent server.
- `zuihitsu <subcommand>`: the operator CLI, a client of the running server (`zuihitsu debug events`, `zuihitsu state memory`, and so on).
- `console/`: the web console (Vite, React, and TypeScript). See `console/CONTRIBUTING.md`.
- `crates/eval/`: the evaluation harness, running scenario-based behavioural tests against a real model.

## Configuration

The agent server reads one **environmental config** file that says where and how this instance runs: its database paths, the model and embedding endpoints, the bind address, and any MCP servers or connectors. It's operational, not behavioural (behavioural settings live in the log as events), and it's per-instance and never committed, since endpoints, credentials, and database paths differ from machine to machine.

Pass it with `--config`. A missing file falls back to defaults: a loopback server writing to `data/` beside the config. Relative paths resolve against the config file's own directory, so you can relocate an instance by moving it. Secrets never cross the read-only config view (`GET /control/config`): control keys serialise as a count, environment variables and HTTP headers as their names alone, and endpoint URLs with their credentials stripped.

A full `config.toml`, every section with its defaults:

```toml
[storage]
# The directory holding the three databases (event log, graph, and vector index),
# resolved relative to this file. This directory *is* the instance selector: two
# configs with different paths are two independent agents. Default: "data".
dir = "data"

[serving]
# The address the long-running server binds. A loopback peer is trusted without a
# key; a remote peer must present a control key. Default: "127.0.0.1:7777".
bind = "127.0.0.1:7777"
# Valid API keys for the operator surface (/control/*). Empty (the default) rejects
# every remote control request, leaving the server loopback-only.
control_keys = []

[model]
# Where to reach the generation model (an OpenAI-compatible endpoint). An empty
# endpoint (the default) means no model is configured.
endpoint = "http://localhost:8080/v1"
llm = "your-model-name"
# The model's context window, in tokens. Required whenever an endpoint is set: the
# API doesn't report it, and the agent derives its compaction budget from it.
# Update it when the model or its context length changes.
context_length = 32768
# Sampling: each is optional; an unset field falls back to the serving layer's
# per-model default, so omit any you don't want to override.
temperature = 0.7
top_p = 0.95
top_k = 40
min_p = 0.05
presence_penalty = 0.0
# Override the serving layer's thinking default; omit to leave it to the backend.
thinking = true

[model.resilience]
# Transport resilience for the model client: operational, never logged (a retry
# the agent never saw emits nothing, so replay never depends on these). Defaults
# shown.
request_timeout_seconds = 300      # whole-request timeout for one backend call
max_attempts = 3                   # first try plus retries of transient failures
backoff_base_ms = 500              # first retry's backoff; each retry doubles it
backoff_max_ms = 10000             # per-retry backoff ceiling
breaker_failure_threshold = 3      # consecutive failures that open the circuit
breaker_open_seconds = 30          # how long an open circuit fails fast

[embedding]
# Where to reach the embedding model, and the dimensionality it produces. An empty
# endpoint (the default) disables semantic search: the vector index is populated
# only when this is set.
endpoint = "http://localhost:8080/v1"
model = "your-embedding-model"
dimensions = 1024
# The embedding model's context window, in tokens. When set, inputs are truncated
# to fit; omit for a backend whose window exceeds any single memory entry.
context_length = 512
request_timeout_seconds = 300      # the same hung-backend guard as the model client

[snapshots]
# Periodic graph checkpoints, so boot restores the latest and replays only the log
# tail. On by default: the graph is always rebuildable from the log, but a
# checkpoint turns a slow cold rebuild into a fast one. Defaults shown.
enabled = true
# dir = "snapshots"                # defaults to a snapshots/ directory beside the graph
check_interval_seconds = 3600      # how often the snapshotter checks whether one is due
min_new_events = 20                # events appended since the last before a new one
keep = 5                           # snapshots retained; older ones are pruned

# MCP servers: one [mcp.<name>] block each. The block name is the mcp.<name>.* Lua
# projection prefix, so it must be a valid Lua identifier. A server has exactly one
# transport: a local stdio subprocess (`command`) or a remote streamable-HTTP
# endpoint (`url`), never both.

[mcp.browser]
# Stdio transport: an executable launched as argv (never shell-split).
command = "mcp-server-browser"
args = ["--headless"]
# env and cwd are optional; env values serialise redacted in the config view.
env = { BROWSER_PROFILE = "default" }
# Project only these tools (omit for the whole catalogue), then drop any denied.
allow = ["navigate", "markdown"]
deny = ["evaluate"]

[mcp.remote]
# Streamable-HTTP transport: the endpoint URL of a remote MCP server.
url = "https://mcp.example.com/mcp"
# Custom headers sent with every request: put credentials here, not in the URL;
# header values serialise redacted in the config view.
headers = { Authorization = "Bearer your-token" }

# Platform connectors: one [platform_connectors.<platform>] entry each. The entry
# key is the platform the connector serves (discord, slack, or direct); a
# /platform/* request bearing that connector's key is scoped to and attributed to
# that platform. The key serialises redacted.
[platform_connectors]
discord = { key = "00000000-0000-0000-0000-000000000000" }
```

## Going deeper

- `docs/overview.md`: the design spec, laying out the goals, the architectural principles, and the map of the per-area documents that cover every subsystem in detail.
- `CONTRIBUTING.md`: the conventions and the checks each change must pass.
- `console/CONTRIBUTING.md`: an onboarding guide to the console's structure and motifs.
