# zuihitsu — design overview

**zuihitsu** is an agent system: the software a single conversational agent runs on. One instance hosts exactly one agent, whose entire life is a single event log read from `seq 0`. The agent itself is unnamed by the system — each operator names their own agent at creation time.

The agent meets people across platforms, remembers what each has said, talks to any of them one-to-one or in a group, and keeps confidences between them. A platform is a free-form label carried on every turn alongside a `platform_user_id`, so a new adapter needs no schema change; today that surface is driven by the web console's direct interface and the operator CLI, with named adapters (Discord and others) to follow. Its whole history is replayable, its schema and logic are designed to evolve, and every consequential decision it makes leaves an auditable trace.

This document frames the system; the detail lives in the area documents listed under [The documents](#the-documents), whose order is the reading order — concatenating them in that order reconstructs the single-document spec.

## Goals

- Remember what's been talked about, across sessions and across participants.
- Surface relevant memories proactively at the start of each conversation.
- Treat privacy and confidence between participants as a first-class concern.
- Recognize one human across multiple platforms once an operator has said they're the same person.
- Provide full auditability and replayability of the agent's own evolution via event sourcing.
- Stay extensible: new event fields, new link relations, new capabilities are additive, not migrations.

## Architectural principles

1. **The event log is the source of truth.** Memories, links, tags, conversations — all derived state. The log is the only thing that survives a wipe.
2. **Append-only at every layer.** Content, links, tags: additions and supersessions, never silent overwrites. Supersession is itself an event.
3. **No privileged participants in the agent's model.** The operator holds the console, not a privileged seat in conversation; the agent treats every participant it meets symmetrically. Deference toward anyone emerges from what's in their memory, not from a flag.
4. **Tellers, not roles, govern visibility.** An entry's audience is determined by who told the agent and what was said, not by who the participant "is" globally.
5. **The schema lives in data, not code.** Link relations and their cardinalities are event-sourced and queryable, modifiable like anything else.
6. **Brief composition is deterministic.** Model-driven work happens at description-regeneration time, not when assembling the contextual brief. The brief is a fast, predictable projection of current state.
7. **Conversation boundaries are real.** The system prompt is frozen at conversation start. Mid-conversation changes (a participant joining) arrive as system messages, not prompt rebuilds.
8. **Errors teach.** Every API failure returns structured suggestions where possible. The agent learns its environment by tripping over it.
9. **One instance, one agent.** No `agent_id` anywhere — the log *is* the agent. A fleet is a fleet of instances; any cross-agent interaction happens at the server boundary, never through shared storage.
10. **One writer, many clients.** Exactly one process — the agent server — touches the event log, the graph, and the model. Every other actor (console, CLI, platform adapters) is a client of one server API. Authority is a property of the client's role, enforced server-side, not a property of where it runs or what a participant types.

## Trust model, in brief

Two postures carry the whole design, stated in full in [Trust and authority](trust-and-authority.md). The **operator is trusted**: a single person runs the instance, owns the event log and the binary, and holds console access — but holds no platform identity and no conversational privilege, so in ordinary conversation the operator is just another participant. **Participants are not trusted with each other**: they have legitimate competing interests, so the visibility machinery keeps one participant's asides about another from reaching their subject, self-model writes are unreachable from platform conversations, and retroactive mutation of another participant's records is structurally gated.

## The documents

1. [Trust and authority](trust-and-authority.md) — the trust model in full; clients, the server boundary, and how authority is enforced.
2. [Data model](data-model.md) — memories, content entries, tags, links, and the relation registry; naming conventions; identity, platform stubs, and cross-platform merging.
3. [Events and storage](events-and-storage.md) — the event vocabulary; the log, the materialized graph, the vector store, snapshots, and faithful replay.
4. [Visibility](visibility.md) — the read-time predicate, its surfaces, write-time defaults, provenance markers, and the disclosure judgment layered above them.
5. [Time](time.md) — bi-temporal entries, temporal references and their resolution, the calendar view, scheduled work, and recency and volatility.
6. [Conversations and briefs](conversations-and-briefs.md) — durable conversations, sessions, and compaction; the frozen system prompt; contextual-brief composition.
7. [The write path](write-path.md) — the inference and embedding backends; description regeneration, arbitration, and link inference off the hot path; concurrency.
8. [The agent loop](agent-loop.md) — the tool protocol, the server API and turn lifecycle, and the Lua API the agent acts through.
9. [Initialization and lifecycle](lifecycle.md) — configuration, genesis, boot, and the imprint interview.
10. [Observability and testing](observability-and-testing.md) — the console and its views, testability seams, and validation via the eval harness.
11. [Known limitations and open questions](limitations.md) — named residual risks and recorded decisions; actionable work lives in the issue tracker.
