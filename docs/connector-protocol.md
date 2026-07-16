# Connector protocol

A connector bridges a platform to the agent server. It delivers participant messages, watches the agent's generation arrive, and posts the reply back. This document covers the HTTP surface: endpoints, request/response shapes, streaming format, and auth.

## Auth

Every `/platform/*` endpoint authenticates with a platform key: `Authorization: Bearer <key>`. Loopback peers are trusted without a credential; remote peers must present one. The platform surface carries no operator authority — a connector acts only as the participants it represents, never as the operator (see [Trust and authority → Clients and the server boundary](trust-and-authority.md#clients-and-the-server-boundary)).

## Delivering messages

### `POST /platform/messages`

Deliver a batch of participant turns and run one agent response cycle. Each message is recorded as a separate participant turn; the agent sees them all and responds once.

**Request body:**

```json
{
  "locator": { "platform": "chat", "scope_path": "room/42" },
  "messages": [
    { "sender": { "platform": "chat", "id": "dave" }, "text": "hello" },
    { "sender": { "platform": "chat", "id": "dave" }, "text": "anyone there?" }
  ],
  "present": [
    { "platform": "chat", "id": "dave" },
    { "platform": "chat", "id": "erin" }
  ]
}
```

- `locator` — the conversation's `(platform, scope_path)` pair. The server resolves (or mints) a conversation and its context memory on first contact.
- `messages` — the inbound batch. Each carries a `sender` — a `PersonId`, the `{ platform, id }` pair the server resolves to `person/<id>@<platform>` — and `text`. A single message is a one-element batch.
- `present` — the `PersonId`s currently in the room. The server resolves each to a participant stub (minting on first contact) and uses the set for the subject-guard and join-brief logic.

**Response body (`200 OK`):**

```json
{
  "outcome": { "Reply": "Hello there, Dave." },
  "participant_turn_ids": ["01J…", "01J…"]
}
```

- `outcome` — the turn's conversational outcome: `{"Reply": "…"}`, `"Silent"`, `"MaxStepsExceeded"`, or `"Deferred"`.
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
  "locator": { "platform": "chat", "scope_path": "room/42" },
  "participant": { "platform": "chat", "id": "erin" }
}
```

`participant` is a `PersonId` — the `{ platform, id }` pair the server resolves to `person/<id>@<platform>`.

### `POST /platform/roster`

Resync the room's roster against a fresh member list. Each newly-arrived member receives a `ParticipantJoined` and a join-brief, exactly as an explicit `join` would. The response reports the diff.

**Request body:**

```json
{
  "locator": { "platform": "chat", "scope_path": "room/42" },
  "present": [
    { "platform": "chat", "id": "dave" },
    { "platform": "chat", "id": "erin" },
    { "platform": "chat", "id": "frank" }
  ]
}
```

`present` is the full set of `PersonId`s currently in the room.

**Response body (`200 OK`):**

```json
{ "joined": [{ "platform": "chat", "id": "frank" }], "departed": 0 }
```

`joined` is the `PersonId`s newly briefed in.

## Writing context

### `POST /platform/context`

Write context entries to a conversation's context memory directly, without running a turn. A connector uses this to write room metadata on first contact, or when the room's name or topic changes.

The conversation is minted on first contact if it doesn't yet exist. This is intentional — a connector can establish context before the first message arrives.

**Request body:**

```json
{
  "locator": { "platform": "chat", "scope_path": "room/42" },
  "connector": "my-connector",
  "entries": [
    { "text": "Room: #general. Topic: Welcome to the project." }
  ]
}
```

- `connector` — identifies the caller in the event log. Context entries are attributed to `EventSource::Connector`, not the agent.
- `entries` — the context text to append to the conversation's context memory.

Returns `204 No Content` on success.

## Projecting a participant's identity

### `POST /platform/participant`

Project a participant's platform identity — the username, display name, and nickname a platform surfaces to other users — onto their `person/*` stub as ordinary public entries, so the agent reads someone's current handles from their profile. The stub is minted on first contact if it doesn't yet exist.

Each attribute either records a new value or clears one, and the connector holds the entry id a prior projection returned for it — so a changed value **supersedes** that entry and a cleared one **retracts** it, with no attribute keying on the server. The connector sends an attribute only when its value changed, tracking the last-seen value and returned id per attribute (a nickname per guild, since it varies by server).

**Request body:**

```json
{
  "participant": { "platform": "chat", "id": "dave" },
  "connector": "my-connector",
  "attributes": [
    { "text": "Chat username: dave1234", "supersedes": null },
    { "text": "Chat nickname in Acme: Dave", "supersedes": "01J…" },
    { "text": null, "supersedes": "01J…" }
  ]
}
```

- `text` — the value to record now, or `null` to clear a value that is no longer set.
- `supersedes` — the entry id a prior projection of this attribute returned, to supersede (on a change) or retract (on a clear); `null` on first contact. A target the agent has since dropped is a no-op — the fresh value still lands.

Returns a JSON array of the new entry id per attribute, in request order: a string for a recorded value, `null` for a cleared one. The connector stores these to supersede on the next change.

## Stream frames

Both streaming endpoints (`/platform/messages/stream` and `/control/events/stream`) use the same wire format: an SSE stream where every event has a `data:` payload that is a JSON `StreamFrame`. No `event:` field is emitted — the frame's type lives inside the JSON (`{"type":"progress",…}`), so a consumer reads SSE events, takes each `data:` field, and deserialises it as a `StreamFrame`.

The `StreamFrame` enum is defined in `zuihitsu-frontend-types` and shared by the server, the `zuihitsu-connector-api` crate, and the console's TypeScript bindings.

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

**`POST /platform/messages/stream`** sends `progress` frames while the agent deliberates, then one terminal frame (`outcome` on success, `error` on failure). The stream ends after the terminal.

**`GET /control/events/stream?from=N`** sends `event` frames for committed events (the snapshot replay from seq `N`, then the live tail), interleaved with `progress` frames for ephemeral generation progress. The stream stays open until the server shuts down or the consumer lags off the broadcast, at which point it emits an `end` frame and closes. Reconnect with `?from=<last seen seq + 1>`.

### Consuming the stream

Parse the SSE wire format (split on blank lines, extract `data:` lines) and JSON-parse each `data:` payload as a `StreamFrame`. The `type` field discriminates the variant. Partial frames (a chunk boundary splitting a JSON object) are buffered until the next blank line.

The `zuihitsu-connector-api` crate provides a `PlatformClient` that handles the HTTP transport, SSE parsing, and `StreamFrame` deserialisation. The console has its own `SseDecoder` in `liveStream.ts` for the same purpose.
