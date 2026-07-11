import { describe, expect, it } from "vitest";

import type { Event } from "../../types/Event.ts";
import type { EventPayload } from "../../types/EventPayload.ts";
import { buildInteractions, contextDenominatorsAt } from "./interactions.ts";

function event(seq: number, payload: EventPayload): Event {
  return { seq, recorded_at: seq * 1_000, source: "Agent", payload };
}

function configSet(seq: number, compaction: Record<string, unknown>): Event {
  return event(seq, {
    type: "ConfigSet",
    settings: { compaction },
    source: "Operator",
  } as unknown as EventPayload);
}

function modelCalled(
  seq: number,
  request: Record<string, unknown> | null,
  turnId = "turn-1",
): Event {
  return event(seq, {
    type: "ModelCalled",
    conversation: "conv-1",
    turn_id: turnId,
    phase: "Step",
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
  } as unknown as EventPayload);
}

const user = (content: string) => ({
  role: "user",
  content,
  tool_calls: [],
  tool_call_id: null,
});

describe("buildInteractions", () => {
  it("reconstructs a base plus continuations with record kinds and slice boundaries", () => {
    const events = [
      modelCalled(1, {
        Base: {
          system: "sys",
          system_sections: [{ kind: "Scaffold", start: 0, end: 3 }],
          messages: [user("one")],
          tools: [],
          tool_choice: "Auto",
          thinking: null,
        },
      }),
      modelCalled(2, { Continuation: { appended_messages: [user("two"), user("three")] } }),
    ];
    const [base, next] = buildInteractions(events, 10);
    expect(base.record).toBe("base");
    expect(base.appendedFrom).toBe(0);
    expect(base.systemSections).toHaveLength(1);
    expect(next.record).toBe("continuation");
    expect(next.appendedFrom).toBe(1);
    expect(next.messages).toHaveLength(3);
    expect(next.system).toBe("sys");
  });

  it("marks a request-less call missing, with an empty prompt", () => {
    const [only] = buildInteractions([modelCalled(1, null)], 10);
    expect(only.record).toBe("missing");
    expect(only.system).toBe("");
    expect(only.systemSections).toEqual([]);
  });

  it("defaults absent recorded sections to empty for pre-capture bases (AC1.3)", () => {
    const [only] = buildInteractions(
      [
        modelCalled(1, {
          Base: {
            system: "old prompt",
            messages: [],
            tools: [],
            tool_choice: "Auto",
            thinking: null,
          },
        }),
      ],
      10,
    );
    expect(only.record).toBe("base");
    expect(only.systemSections).toEqual([]);
  });
});

describe("contextDenominatorsAt", () => {
  it("returns the latest ConfigSet at or before the cursor (AC5.5)", () => {
    const events = [
      configSet(1, { token_budget: 24_000, context_length: null }),
      configSet(5, { token_budget: 163_840, context_length: 204_800 }),
      configSet(9, { token_budget: 100, context_length: 200 }),
    ];
    expect(contextDenominatorsAt(events, 6)).toEqual({
      budget: 163_840,
      contextLength: 204_800,
    });
  });

  it("yields nulls for a log with no ConfigSet — budget unknown, never a default", () => {
    expect(contextDenominatorsAt([], 10)).toEqual({ budget: null, contextLength: null });
  });

  it("treats an old-shape snapshot without context_length as unknown, budget still real", () => {
    const events = [configSet(1, { token_budget: 24_000 })];
    expect(contextDenominatorsAt(events, 10)).toEqual({ budget: 24_000, contextLength: null });
  });
});
