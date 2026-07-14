import { describe, expect, it } from "vitest";

import type { Event } from "@zuihitsu/wire/types/Event.ts";
import type { LiveEvent } from "@zuihitsu/wire/types/LiveEvent.ts";
import type { TurnProgress } from "@zuihitsu/wire/types/TurnProgress.ts";
import { fold, runningKey, type LiveEval } from "./liveEval.ts";

const KEY = runningKey(0, 0);

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

function runProgress(overrides: Partial<TurnProgress>): LiveEvent {
  return { kind: "run_progress", scenario: 0, run: 0, frame: frame(overrides) } as LiveEvent;
}

function runEvent(payload: object): LiveEvent {
  return {
    kind: "run_event",
    scenario: 0,
    run: 0,
    event: { seq: 9, recorded_at: 9, source: "Agent", payload } as unknown as Event,
  } as LiveEvent;
}

function streaming(): LiveEval {
  let state: LiveEval = {
    pkg: null,
    status: { status: "streaming" },
    liveRuns: new Map([[KEY, []]]),
    progress: new Map(),
  };
  state = fold(state, runProgress({ kind: "reply", text: "streamed" }));
  return state;
}

/// The fold owns the mark-not-delete supersede wiring: a mid-turn `ModelCalled` must leave the
/// accumulation in place (marked), because that object is what holds the pending turn's transcript
/// slot while the deep-dive's replica refolds — deleting here is the disclosure-collapsing remount
/// bug the design exists to prevent.
describe("the live eval fold's supersede wiring", () => {
  it("a mid-turn ModelCalled marks the run's accumulation and keeps it", () => {
    const state = fold(
      streaming(),
      runEvent({ type: "ModelCalled", conversation: "c1", turn_id: "t1" }),
    );
    expect(state.progress.get(KEY)?.get("c1")).toMatchObject({
      reply: "streamed",
      superseded: true,
    });
  });

  it("the agent's ConversationTurn drops the accumulation", () => {
    const state = fold(
      streaming(),
      runEvent({ type: "ConversationTurn", conversation: "c1", turn_id: "t1", role: "Agent" }),
    );
    expect(state.progress.get(KEY)?.has("c1")).toBe(false);
  });

  it("an abandoned frame drops the accumulation — a deferral commits no event to do it", () => {
    const state = fold(streaming(), runProgress({ kind: "abandoned", text: "retries exhausted" }));
    expect(state.progress.get(KEY)?.has("c1")).toBe(false);
  });
});
