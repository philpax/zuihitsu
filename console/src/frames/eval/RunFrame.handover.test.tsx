// @vitest-environment jsdom
import { beforeAll, describe, expect, it, vi } from "vitest";
import { act, StrictMode } from "react";
import { createRoot, type Root } from "react-dom/client";

import type { Event } from "@zuihitsu/wire/types/Event.ts";
import type { PackageSummary } from "@zuihitsu/wire/types/PackageSummary.ts";
import type { RunRecord } from "@zuihitsu/wire/types/RunRecord.ts";
import type { RunSummary } from "@zuihitsu/wire/types/RunSummary.ts";
import type { EvalContext } from "../../lib/api/liveEval.ts";
import type { InFlightGeneration } from "../../lib/model/inflight.ts";
import { RunFrame } from "./RunFrame.tsx";
import { EvalRouteContext } from "./evalContext.ts";

// The frame is exercised for its handover behaviour, not routing: stub the router hooks so it renders
// as an ordinary React child, reading a fixed run coordinate from `useParams` and its package from the
// `EvalRouteContext` the test provides. Re-rendering with fresher events updates the same `RunFrame`
// instance in place (preserving its disclosure state) rather than remounting it. `Link` becomes a
// plain anchor; `Navigate` (the not-found redirect) renders nothing, since the fixture resolves.
vi.mock("@tanstack/react-router", async () => {
  const { createElement } = await import("react");
  return {
    useParams: () => ({ scenario: "scenario-a", run: "0", view: "conversation" }),
    useSearch: () => ({}),
    useNavigate: () => () => {},
    useLocation: () => ({ pathname: "/eval/scenario-a/0/conversation" }),
    Link: ({ children, className, title }: Record<string, unknown>) =>
      createElement("a", { className, title }, children as never),
    Navigate: () => null,
  };
});

// Only the wasm boundary is mocked: `Replica.fromEvents` yields a fresh query stub per call — a new
// instance per refold, exactly as production behaves — and the ref scanner scans nothing. The rest
// of the stack (`useReplica` included) is the real code under test.
vi.mock("../../lib/replica/replica.ts", async (importOriginal) => ({
  ...(await importOriginal<object>()),
  scanTurnRefs: (text: string) => [{ kind: "prose", text }],
  Replica: {
    fromEvents: (events: Event[]) =>
      Promise.resolve({
        headSeq: events.length > 0 ? events[events.length - 1].seq : 0,
        foldedSeq: events.length > 0 ? events[events.length - 1].seq : 0,
        foldTo() {},
        memories: () => [],
        conversations: () => [],
        participantIds: () => [],
        requestDigests: () => [],
      }),
  },
}));

beforeAll(() => {
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  window.matchMedia ??= ((query: string) => ({
    matches: false,
    media: query,
    onchange: null,
    addListener: () => {},
    removeListener: () => {},
    addEventListener: () => {},
    removeEventListener: () => {},
    dispatchEvent: () => false,
  })) as typeof window.matchMedia;
  globalThis.ResizeObserver ??= class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
});

function event(seq: number, payload: unknown): Event {
  return { seq, recorded_at: 1_000 + seq, source: "Agent", payload } as unknown as Event;
}

const beforeCommit: Event[] = [
  event(1, {
    type: "ConversationStarted",
    id: "conv-1",
    locator: { platform: "test", scope_path: "room" },
    context_memory: null,
  }),
  event(2, {
    type: "SessionStarted",
    id: "sess-1",
    conversation: "conv-1",
    brief: "a room",
    started_at: 1_000,
    participants: [],
    seeded_from_turn: null,
    working_set: [],
  }),
  event(3, {
    type: "ConversationTurn",
    conversation: "conv-1",
    turn_id: "U1",
    role: "Participant",
    text: "hello?",
    participant: null,
    initiation: "Responding",
    brief: null,
  }),
];

// Appended, so the earlier elements keep their identity — the shape the live fold guarantees and
// the same-run heuristic in `useReplica` relies on.
const afterCommit: Event[] = [
  ...beforeCommit,
  event(4, {
    type: "ModelCalled",
    conversation: "conv-1",
    turn_id: "T1",
    phase: "Step",
    reasoning: "pondered the reply",
    completion: { Reply: "hello" },
    finish_reason: null,
    duration_ms: 100,
    usage: {
      prompt_tokens: null,
      completion_tokens: null,
      total_tokens: null,
      cache_read_tokens: null,
      cache_write_tokens: null,
    },
    request: null,
  }),
];

const generating: InFlightGeneration = {
  turnId: "T1",
  step: 0,
  phase: "Step",
  reasoning: "pondering the reply",
  reply: "",
  restarts: 0,
  superseded: false,
};

const pkg = {
  meta: {
    harness_version: "test",
    git_sha: null,
    git_dirty: false,
    model_id: "test-model",
    embedding_model: null,
    scenario_filter: null,
    started_at_ms: 0,
    finished_at_ms: 0,
    runs_per_scenario: 1,
    concurrency: 1,
    rejudged_from: null,
    resumed_from: null,
  },
  scenarios: [
    {
      meta: {
        name: "scenario-a",
        category: "Recall",
        description: "a scenario",
        bar: { kind: "gating", min_rate: 1 },
      },
      runs: [],
      aggregate: {
        runs: 0,
        rate: 0,
        gating_passed: true,
        gating_rate: 1,
        wall_clock_ms: { p50: 0, p95: 0, mean: 0 },
        latency_ms: { p50: 0, p95: 0, mean: 0 },
        tokens: { prompt_mean: 0, completion_mean: 0 },
        steps_mean: 0,
      },
    },
  ],
} as unknown as PackageSummary;

// The lean summary the fold lands when the run completes — verdicts and metrics, no event log.
const summary: RunSummary = {
  index: 0,
  started_at_ms: 0,
  finished_at_ms: 0,
  verdicts: [],
  metrics: {
    model_calls: 1,
    steps: 1,
    wall_clock_ms: 0,
    total_latency_ms: 100,
    prompt_tokens: 0,
    completion_tokens: 0,
    total_tokens: 0,
    gating_passed: true,
  },
  usages: [],
} as unknown as RunSummary;

// A package whose only run has completed — its summary is folded in, and the deep-dive fetches the
// full record on demand.
const completedPkg = {
  ...pkg,
  scenarios: [{ ...pkg.scenarios[0], runs: [summary] }],
} as unknown as PackageSummary;

/// A never-called `getRun` for the tests where the run stays live and no record is ever fetched.
function neverFetch(): Promise<RunRecord> {
  return Promise.reject(new Error("getRun should not be called while the run is live"));
}

function context(
  events: Event[] | null,
  progress: ReadonlyMap<string, InFlightGeneration>,
  overrides: Partial<EvalContext> = {},
): EvalContext {
  return {
    pkg,
    liveRuns: events ? new Map([["0:0", events]]) : new Map(),
    live: { status: "streaming" },
    progress: new Map([["0:0", progress]]),
    getRun: neverFetch,
    ...overrides,
  };
}

async function render(root: Root, ctx: EvalContext) {
  await act(async () => {
    root.render(
      <StrictMode>
        <EvalRouteContext.Provider value={ctx}>
          <RunFrame />
        </EvalRouteContext.Provider>
      </StrictMode>,
    );
  });
}

function disclosureButton(container: HTMLElement): HTMLButtonElement {
  const button = [...container.querySelectorAll("button")].find((candidate) =>
    candidate.textContent?.includes("deliberation"),
  );
  if (!button) throw new Error("no deliberation disclosure rendered");
  return button;
}

describe("the run frame across the first step's commit", () => {
  it("keeps an opened deliberation open through the event landing and the replica refolding", async () => {
    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);

    // Streaming: the run is live, tokens are arriving, the reader opens the deliberation.
    await render(root, context(beforeCommit, new Map([["conv-1", generating]])));
    act(() => disclosureButton(container).click());
    expect(container.textContent).toContain("pondering the reply");

    // The step commits: the run's log grows, the supersede marks the in-flight accumulation (the
    // fold keeps the object so the pending turn holds its slot while the replica refolds
    // asynchronously — deleting it here is exactly the remount bug this test pins), and the new
    // replica instance lands. The disclosure must ride through all of it.
    await render(
      root,
      context(afterCommit, new Map([["conv-1", { ...generating, superseded: true }]])),
    );
    expect(container.textContent).toContain("pondered the reply");

    act(() => root.unmount());
    container.remove();
  });
});

describe("the run frame when a live run completes", () => {
  it("keeps the last live events rendered until the fetched record arrives", async () => {
    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);

    // Streaming: the run is live, the reader opens the deliberation.
    await render(
      root,
      context(afterCommit, new Map([["conv-1", { ...generating, superseded: true }]])),
    );
    act(() => disclosureButton(container).click());
    expect(container.textContent).toContain("pondered the reply");

    // The run completes: the fold retires the live buffer and lands the summary, but the record fetch
    // is still in flight. The open view must keep the last live events rendered — not redirect, not
    // flash to loading — so the deliberation stays put through the handover.
    let resolveRecord: (record: RunRecord) => void = () => {};
    const pending = new Promise<RunRecord>((resolve) => {
      resolveRecord = resolve;
    });
    await render(root, context(null, new Map(), { pkg: completedPkg, getRun: () => pending }));
    expect(container.textContent).toContain("pondered the reply");

    // The record lands: its full log takes over (the shared first event keeps the same run on screen),
    // and the deliberation is still there.
    const record = {
      index: 0,
      started_at_ms: 0,
      finished_at_ms: 0,
      events: [...afterCommit],
      journal: [],
      verdicts: [],
      metrics: summary.metrics,
    } as unknown as RunRecord;
    await act(async () => {
      resolveRecord(record);
      await pending;
    });
    expect(container.textContent).toContain("pondered the reply");

    act(() => root.unmount());
    container.remove();
  });
});
