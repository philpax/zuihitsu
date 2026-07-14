import { describe, expect, it } from "vitest";

import type { Event } from "@zuihitsu/wire/types/Event.ts";
import type { EventPayload } from "@zuihitsu/wire/types/EventPayload.ts";
import { deriveContextDebug } from "./contextDebug.ts";

function modelCalled(
  seq: number,
  phase: "Step" | "Synthesis",
  request: Record<string, unknown> | null,
  turnId: string,
): Event {
  return {
    seq,
    recorded_at: seq * 1_000,
    source: "Agent",
    payload: {
      type: "ModelCalled",
      conversation: "conv-1",
      turn_id: turnId,
      phase,
      request_digest: "digest",
      request,
      completion: { Reply: "ok" },
      reasoning: null,
      finish_reason: null,
      usage: {
        prompt_tokens: null,
        completion_tokens: null,
        total_tokens: null,
        cache_read_tokens: null,
        cache_write_tokens: null,
      },
      duration_ms: 5,
    } as unknown as EventPayload,
  };
}

const user = (content: string) => ({
  role: "user",
  content,
  tool_calls: [],
  tool_call_id: null,
});

function base(system: string, messages: unknown[]) {
  return {
    Base: {
      system,
      system_sections: [],
      messages,
      tools: [],
      tool_choice: "Auto",
      thinking: null,
    },
  };
}

describe("deriveContextDebug", () => {
  it("keeps a Step chain warm across an interleaved Synthesis call", () => {
    // A synthesis pass is a separate structured request in the same conversation; it must not
    // read as a system change inside the Step chain, nor break its measured deltas.
    const events = [
      modelCalled(1, "Step", base("conversational system", [user("one")]), "turn-1"),
      modelCalled(2, "Synthesis", base("summarize this memory", [user("Memory: x")]), "turn-1"),
      modelCalled(3, "Step", base("conversational system", [user("one"), user("two")]), "turn-2"),
    ];
    const { verdictBySeq } = deriveContextDebug(events, 10);
    expect(verdictBySeq.get(3)?.path).toBe("warm");
    expect(verdictBySeq.get(2)?.cause).toBe("first-call");
  });
});
