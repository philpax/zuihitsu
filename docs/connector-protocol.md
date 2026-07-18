# Connector protocol

A connector bridges a platform to the agent server. It delivers participant messages, watches the agent's generation arrive, and posts the reply back. This document covers the HTTP surface: endpoints, request/response shapes, streaming format, and auth.

## Auth

Each connector is registered in the instance config under a top-level `[connectors]` map. The map key is the connector's id, which is both its platform id and the id its writes are attributed to; the value carries its bearer key:

```toml
[connectors]
discord = { key = "totally secret key!" }
```

Every `/platform/*` request is scoped to exactly one connector, and the key decides the scope first. A request bearing a registered connector's key (`Authorization: Bearer <key>`) resolves to that connector's id — wherever it connects from, so a connector running on the same host as the server (a bot on `localhost`, the usual deployment) is still scoped to its own platform by its key, not mistaken for the operator's console. That id is the platform every operation in the request acts on, and the connector its writes are attributed to. A request bearing no key falls back to its origin: a loopback peer is the operator's own console, scoped to the reserved `direct` platform; a remote peer is rejected with `401`. A request bearing an unrecognised key is rejected with `401` outright — a misconfigured connector fails loudly rather than silently acting as `direct`.

Because the platform is derived from the connector's key, no request body carries one — there is nothing in the payload to spoof. The platform surface carries no operator authority: a connector acts only as the participants it represents, never as the operator (see [Trust and authority → Clients and the server boundary](trust-and-authority.md#clients-and-the-server-boundary)).

## Delivering messages

### `POST /platform/messages`

Deliver a batch of participant turns and run one agent response cycle. Each message is recorded as a separate participant turn; the agent sees them all and responds once.

**Request body:**

```json
{
  "scope_path": "room/42",
  "messages": [
    { "sender": "dave", "text": "hello" },
    { "sender": "dave", "text": "anyone there?" }
  ],
  "present": ["dave", "erin"]
}
```

- `scope_path` — the conversation's address within the connector's platform. The server pairs it with the request's connector platform, and resolves (or mints) a conversation and its context memory on first contact.
- `messages` — the inbound batch. Each carries a `sender` — the bare id of the sender, resolved under the request's connector platform to `person/<id>@<platform>` — and `text`. A single message is a one-element batch.
- `present` — the bare ids currently in the room, each resolved under the request's connector platform. The server resolves each to a participant stub (minting on first contact) and uses the set for the subject-guard and join-brief logic.

**Response body (`200 OK`):**

```json
{
  "outcome": { "Reply": "Hello there, Dave." },
  "participant_turn_ids": ["01J…", "01J…"]
}
```

- `outcome` — the turn's conversational outcome: `{"Reply": "…"}`, `"Silent"`, `"MaxStepsExceeded"`, `"Deferred"`, or `"Superseded"`. `"Superseded"` means a newer inbound batch arrived for the same conversation while this turn was generating: the newer batch's turn answers once with everything in context, so no reply comes via this request — a connector treats it like `"Silent"`. The `participant_turn_ids` still carry this batch's recorded inbound turns, so the connector can map its message ids even though the messages were folded into the successor's answer.
- `participant_turn_ids` — the durable turn ids (Crockford ULID strings), one per inbound message. A connector uses these to map its own message ids to zuihitsu turns, so it can inject a `[turn:<id>]` token when a user replies to one of those messages later.

Returns `503` if no model is configured.

### `POST /platform/messages/stream`

The streamed sibling of `/platform/messages`. Same request body; the response is an SSE stream of `StreamFrame` values (see [Stream frames](#stream-frames)).

A connector uses this to drive a typing indicator or partial-message edits. The stream ends after the terminal frame (`outcome` or `error`). A connector that ignores every `progress` frame behaves identically to one that never upgraded.

## Noting presence

### `POST /platform/join`

Note a participant arriving mid-session. If the room has a live session, this records a `ParticipantJoined` and injects the joiner's brief. A no-op if the room has never been seen or has no live session — the next message opens a session with the joiner present.

**Request body:**

```json
{
  "scope_path": "room/42",
  "participant": "erin"
}
```

- `scope_path` — the room's address within the connector's platform.
- `participant` — the bare id of the joiner, resolved under the request's connector platform to `person/<id>@<platform>`.

### `POST /platform/roster`

Resync the room's roster against a fresh member list. Each newly-arrived member receives a `ParticipantJoined` and a join-brief, exactly as an explicit `join` would. The response reports the diff.

**Request body:**

```json
{
  "scope_path": "room/42",
  "roster": ["dave", "erin", "frank"]
}
```

- `scope_path` — the room's address within the connector's platform.
- `roster` — the full set of bare ids currently in the room, each resolved under the request's connector platform.

**Response body (`200 OK`):**

```json
{ "joined": ["frank"], "departed": 0 }
```

`joined` is the bare ids newly briefed in.

## Writing context

### `POST /platform/context`

Write context entries to a conversation's context memory directly, without running a turn. A connector uses this to write room metadata on first contact, or when the room's name or topic changes.

The conversation is minted on first contact if it doesn't yet exist. This is intentional — a connector can establish context before the first message arrives.

**Request body:**

```json
{
  "scope_path": "room/42",
  "entries": [
    { "text": "Room: #general. Topic: Welcome to the project." }
  ]
}
```

- `scope_path` — the room's address within the connector's platform.
- `entries` — the context text to append to the conversation's context memory. The entries are attributed to `EventSource::Connector` — the request's connector, from its key — not the agent.

Returns `204 No Content` on success.

## Projecting attributes

### `POST /platform/project`

Project platform attributes onto a scoped memory as ordinary public entries: a participant's identity — the username, display name, and nickname a platform surfaces — onto their `person/*` stub, or a guild's name onto its `context/*` memory. The target is minted on first contact if it doesn't yet exist.

Each attribute either records a new value or clears one, and the connector holds the entry id a prior projection returned for it — so a changed value **supersedes** that entry and a cleared one **retracts** it, with no attribute keying on the server. The connector sends an attribute only when its value changed, tracking the last-seen value and returned id per `(subject, attribute)` (a nickname per guild, since it varies by server).

**Request body:**

```json
{
  "target": { "participant": { "id": "dave" } },
  "attributes": [
    { "text": "Chat username: dave1234", "supersedes": null },
    { "text": "Chat nickname in Acme: Dave", "supersedes": "01J…" },
    { "text": null, "supersedes": "01J…" }
  ]
}
```

- `target` — the memory to project onto, scoped to the request's connector: `{ "participant": { "id": … } }` (resolved to `person/<id>@<platform>`) or `{ "context": { "scope_path": … } }` (resolved to that scope's `context/*` memory, e.g. `guild/42` for a guild's name).
- `text` — the value to record now, or `null` to clear a value that is no longer set.
- `supersedes` — the entry id a prior projection of this attribute returned, to supersede (on a change) or retract (on a clear); `null` on first contact. A target the agent has since dropped is a no-op — the fresh value still lands.

Returns a JSON array of the new entry id per attribute, in request order: a string for a recorded value, `null` for a cleared one. The connector stores these to supersede on the next change.

## Linking scoped memories

### `POST /platform/link`

Assert — or, with `remove`, retract — a structural link between two of the connector's own scoped memories. A connector uses this to record placement: a channel and its members are `part_of` a guild, say. Both endpoints are resolved under the request's connector, so a connector can only ever link memories it owns.

Each endpoint is either a participant (by bare id, resolved to `person/<id>@<platform>`) or a context (by scope path, resolved to that scope's `context/*` memory). On assert, a missing endpoint is minted, so a link lands even on first sight of the guild or member; on retract, the endpoints are resolved without minting, so a retract naming an unknown node is a no-op rather than a pointless mint.

**Request body:**

```json
{
  "from": { "participant": { "id": "dave" } },
  "to": { "context": { "scope_path": "guild/42" } },
  "relation": "part_of",
  "remove": false
}
```

- `from`, `to` — the link's endpoints. Each is `{ "participant": { "id": … } }` or `{ "context": { "scope_path": … } }`, scoped to the request's connector.
- `relation` — the link relation, which must be registered in the ontology (`part_of`, for placement). `same_as` is refused: cross-platform identity is operator-asserted, never a connector's to assert.
- `remove` — `false` (or omitted) to assert the link, `true` to retract it.

The edge is `Public` (a structural fact, not a told aside) and carries `LinkSource::Connector` — the request's connector, from its key — so an audit reads which connector authored it.

Returns `204 No Content` on success. An unregistered relation or an attempt at `same_as` is a `400`.

## Stream frames

Both streaming endpoints (`/platform/messages/stream` and `/control/events/stream`) use the same wire format: an SSE stream where every event has a `data:` payload that is a JSON `StreamFrame`. No `event:` field is emitted — the frame's type lives inside the JSON (`{"type":"progress",…}`), so a consumer reads SSE events, takes each `data:` field, and deserialises it as a `StreamFrame`.

The `StreamFrame` enum is defined in `zuihitsu-frontend-types` and shared by the server, the `zuihitsu-platform-connector-api` crate, and the console's TypeScript bindings.

### Wire format

Each SSE event is one `data:` line carrying a JSON object, terminated by a blank line:

```text
data: {"type":"progress","conversation":"01J…","turn_id":"01J…","phase":"reply","kind":"reply","text":"Hello","step":0}

data: {"type":"outcome","outcome":{"Reply":"Hello there, Dave."},"participant_turn_ids":["01J…"]}
```

### Frame types

| `type` | Payload | Endpoints | Terminal? |
|---|---|---|---|
| `progress` | `TurnProgress` — one fragment of an in-flight generation (reasoning, reply, restart, or abandoned). Ephemeral; never stored. | both | no |
| `event` | `Event` — a committed event appended to the store. The `seq` field is the monotonic cursor a consumer tracks for reconnection. | `/control/events/stream` | no |
| `outcome` | `PlatformResponse` — the turn completed. Same response as the unary endpoint. | `/platform/messages/stream` | yes |
| `end` | (none) — the server is closing the stream (shutdown or broadcast lag). Reconnect from the last seen seq. | `/control/events/stream` | yes |
| `error` | `{ message: string }` — a turn failure. | `/platform/messages/stream` | yes |

### Endpoint behaviour

**`POST /platform/messages/stream`** sends `progress` frames while the agent deliberates, then one terminal frame (`outcome` on success, `error` on failure). The stream ends after the terminal. When a newer batch supersedes this request's turn, the stream terminates promptly with a normal `outcome` frame carrying `"Superseded"` — well before the successor completes, since the successor answers with everything in context through its own request.

**`GET /control/events/stream?from=N`** sends `event` frames for committed events (the snapshot replay from seq `N`, then the live tail), interleaved with `progress` frames for ephemeral generation progress. The stream stays open until the server shuts down or the consumer lags off the broadcast, at which point it emits an `end` frame and closes. Reconnect with `?from=<last seen seq + 1>`.

### Consuming the stream

Parse the SSE wire format (split on blank lines, extract `data:` lines) and JSON-parse each `data:` payload as a `StreamFrame`. The `type` field discriminates the variant. Partial frames (a chunk boundary splitting a JSON object) are buffered until the next blank line.

The `zuihitsu-platform-connector-api` crate provides a `PlatformClient` that handles the HTTP transport, SSE parsing, and `StreamFrame` deserialisation. The console has its own `SseDecoder` in `liveStream.ts` for the same purpose.
