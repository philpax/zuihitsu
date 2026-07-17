import { describe, expect, it } from "vitest";

import type { Event } from "@zuihitsu/wire/types/Event.ts";
import type { EventPayload } from "@zuihitsu/wire/types/EventPayload.ts";
import { buildConversations } from "./conversation.ts";

function event(seq: number, payload: EventPayload): Event {
  return { seq, recorded_at: seq, source: "Orchestration", payload } as Event;
}

const started = event(2, {
  type: "ConversationStarted",
  id: "conv-1",
  locator: { platform: "console", scope_path: "lua" },
  context_memory: "room-1",
} as EventPayload);

describe("buildConversations", () => {
  it("includes a conversation the graph lists as live", () => {
    const conversations = buildConversations([started], new Map(), new Set(["conv-1"]));
    expect(conversations.map((c) => c.id)).toEqual(["conv-1"]);
  });

  it("drops a conversation the graph no longer lists as live", () => {
    // The graph fold drops a conversation whose room memory was deleted, so `replica.conversations()`
    // omits it, its id is absent from the live set, and the transcript excludes it — even though its
    // ConversationStarted event is still in the log.
    const conversations = buildConversations([started], new Map(), new Set());
    expect(conversations).toEqual([]);
  });
});
