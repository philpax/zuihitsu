import { describe, expect, it } from "vitest";

import type { TurnProgress } from "@zuihitsu/wire/types/TurnProgress.ts";
import { foldFrame, supersede, supersededConversation } from "./inflight.ts";
import type { InFlightGeneration } from "./inflight.ts";

/// Fold a frame that must produce an accumulation (everything but `abandoned`).
function fold(
  current: InFlightGeneration | undefined,
  next: Parameters<typeof foldFrame>[1],
): InFlightGeneration {
  const folded = foldFrame(current, next);
  if (!folded) throw new Error("expected an accumulation");
  return folded;
}

function frame(overrides: Partial<TurnProgress>): TurnProgress {
  return {
    conversation: "c1",
    turn_id: "t1",
    phase: "Step",
    kind: "reply",
    text: "",
    step: 0,
    ...overrides,
  } as TurnProgress;
}

describe("foldFrame", () => {
  it("accumulates text and returns fresh identities per frame", () => {
    const first = fold(undefined, frame({ kind: "reasoning", text: "Thinking " }));
    const second = fold(first, frame({ kind: "reply", text: "Hello" }));
    expect(second.reasoning).toBe("Thinking ");
    expect(second.reply).toBe("Hello");
    expect(second).not.toBe(first);
  });

  it("starts fresh when the step advances", () => {
    const first = fold(undefined, frame({ text: "old" }));
    const next = fold(first, frame({ step: 1, text: "new" }));
    expect(next.reply).toBe("new");
    expect(next.step).toBe(1);
  });

  it("an abandoned frame drops the accumulation — the generation has no durable successor", () => {
    const first = fold(undefined, frame({ text: "doomed" }));
    expect(
      foldFrame(first, frame({ kind: "abandoned", text: "retries exhausted" })),
    ).toBeUndefined();
  });

  it("a restart voids the accumulation and counts the retry", () => {
    const first = fold(undefined, frame({ text: "doomed" }));
    const restarted = fold(first, frame({ kind: "restart", text: "connection reset" }));
    expect(restarted.reply).toBe("");
    expect(restarted.restarts).toBe(1);
  });

  it("a new phase or step starts a fresh, unsuperseded accumulation", () => {
    const stepDone = { ...fold(undefined, frame({ text: "done" })), superseded: true };
    const nextStep = fold(stepDone, frame({ step: 1, text: "next" }));
    expect(nextStep.superseded).toBe(false);
    expect(nextStep.reply).toBe("next");
    const synthesis = fold(stepDone, frame({ phase: "Synthesis", text: "naming" }));
    expect(synthesis.superseded).toBe(false);
    expect(synthesis.phase).toBe("Synthesis");
  });
});

describe("supersede", () => {
  const generation = {
    turnId: "t1",
    step: 0,
    phase: "Step",
    reasoning: "",
    reply: "streamed",
    restarts: 0,
    superseded: false,
  } as const;
  const event = (payload: object) => ({ seq: 1, payload }) as never;

  it("a mid-turn ModelCalled marks the accumulation rather than dropping it", () => {
    // The object must survive to hold the pending turn's transcript slot while the view's cursor
    // catches up with the commit; only its display yields.
    const next = supersede(generation, event({ type: "ModelCalled", conversation: "c1" }));
    expect(next).toMatchObject({ reply: "streamed", superseded: true });
  });

  it("the agent's ConversationTurn drops the accumulation", () => {
    expect(
      supersede(generation, event({ type: "ConversationTurn", conversation: "c1", role: "Agent" })),
    ).toBeUndefined();
  });
});

describe("supersededConversation", () => {
  it("fires on a committed ModelCalled and the agent's turn, not a participant's", () => {
    const event = (payload: object) => ({ seq: 1, payload }) as never;
    expect(supersededConversation(event({ type: "ModelCalled", conversation: "c1" }))).toBe("c1");
    expect(
      supersededConversation(
        event({ type: "ConversationTurn", conversation: "c1", role: "Agent" }),
      ),
    ).toBe("c1");
    expect(
      supersededConversation(
        event({ type: "ConversationTurn", conversation: "c1", role: "Participant" }),
      ),
    ).toBeNull();
  });
});
