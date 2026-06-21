# Model management

zuihitsu reaches its model through an OpenAI-compatible endpoint. That API exposes the model's name but not its **context window**, so the operator states the window in config, and the agent sizes its compaction budget from it.

## Configuring a model

The `[model]` section of `config.toml` points at the endpoint and names the model:

```toml
[model]
endpoint = "http://localhost:8080/v1"
llm = "gemma-4-31b"
context_length = 262144
```

`context_length` is the model's context window, in tokens. It is **required whenever an endpoint is set** — the server refuses to start without it, since the agent cannot ask the endpoint for it. An instance with no model endpoint (a test or an offline build) needs no `context_length`.

## The compaction budget

A long conversation eventually fills the context window. Before it does, the agent re-segments — it ends the session, flushes anything worth keeping to memory, and opens a fresh one with a compact brief (see the spec, §Compaction). The threshold it watches is the **compaction budget**: when a turn's prompt crosses it, the session compacts.

The budget is derived from the context window at agent creation:

    compaction budget = floor(context_length × 0.8)

The 0.8 leaves headroom under the window for the system prefix and the reply. The derived value is written into the agent's log as its initial setting, so the compaction trigger and the console both read a single, concrete number; an explicit settings override (via `set-settings`) still wins if you want a different budget.

## Changing the model or its context window

Because the budget is derived once, at creation, an existing agent keeps the budget it was born with. If you change `context_length` — a larger window, or a different backing model — update it in `config.toml`, and then re-derive the agent's budget if you want it to track the new window (push an updated setting; the budget is `0.8 × context_length`). New agents created after the change get the new budget automatically.

The evaluation harness is unaffected: its compaction scenarios set their own tight budget explicitly, rather than relying on a model's window.
