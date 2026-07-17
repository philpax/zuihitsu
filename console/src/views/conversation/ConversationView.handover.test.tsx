// @vitest-environment jsdom
import { beforeAll, describe, expect, it, vi } from "vitest";
import { act, StrictMode } from "react";
import { createRoot, type Root } from "react-dom/client";

import type { Event } from "@zuihitsu/wire/types/Event.ts";
import type { InFlightGeneration } from "../../lib/model/inflight.ts";
import type { Replica } from "../../lib/replica/replica.ts";
import { RouterProvider } from "../../lib/nav/history.tsx";
import { ConversationView } from "./ConversationView.tsx";

// The view is exercised for its own behaviour, not routing — but the real (synchronous) router runtime
// is light, so mount it at a live stream URL rather than stubbing: the view reads its frame from the
// location and renders as an ordinary child (state preserved across re-renders, props flowing).

// The wasm bridge needs a browser fetch to initialise; under jsdom the ref scanner is stubbed to
// "no references", which every fixture text here satisfies.
vi.mock("../../lib/replica/replica.ts", async (importOriginal) => ({
  ...(await importOriginal<object>()),
  scanTurnRefs: (text: string) => [{ kind: "prose", text }],
}));

// jsdom lacks the browser APIs motion/react feature-detects; stub them before anything renders.
beforeAll(() => {
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  // The stream views resolve their frame from the address bar; sit at the live conversation.
  window.history.replaceState(null, "", "/live/conversation");
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
  // The at-head view follows its foot as the in-flight generation streams in; jsdom's own `scrollTo`
  // only logs "Not implemented", so replace it outright with a no-op.
  window.scrollTo = () => {};
});

/// The read surface the view actually queries during render; everything else is behind click
/// handlers this test never fires.
const replica = {
  memories: () => [],
  conversations: () => [],
  participantIds: () => [],
  requestDigests: () => [],
} as unknown as Replica;

function event(seq: number, payload: unknown): Event {
  return { seq, recorded_at: 1_000 + seq, source: "Agent", payload } as unknown as Event;
}

/// The log as it stands while the agent's first step is still streaming: the room opened and the
/// participant spoke, but no deliberation event has committed.
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

/// The same log the render after the first step's commit sees: the `ModelCalled` landed, which both
/// materialises the turn in the fold and supersedes the in-flight accumulation.
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

async function render(
  root: Root,
  events: Event[],
  cursor: number,
  progress: ReadonlyMap<string, InFlightGeneration>,
) {
  await act(async () => {
    root.render(
      <StrictMode>
        <RouterProvider mode="console">
          <ConversationView
            replica={replica}
            events={events}
            cursor={cursor}
            atHead
            progress={progress}
          />
        </RouterProvider>
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

/// A second room opening later, with fresher activity than the first — the shape that used to yank
/// the default selection over to it.
const secondRoom: Event[] = [
  event(10, {
    type: "ConversationStarted",
    id: "conv-2",
    locator: { platform: "test", scope_path: "annex" },
    context_memory: null,
  }),
  event(11, {
    type: "ConversationTurn",
    conversation: "conv-2",
    turn_id: "U2",
    role: "Participant",
    text: "over here!",
    participant: null,
    initiation: "Responding",
    brief: null,
  }),
];

describe("the default room selection", () => {
  it("stays on the pinned room when a fresher room appears, and marks the working room", async () => {
    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);

    // One room: the default pins to it.
    await render(root, beforeCommit, 3, new Map());
    expect(container.textContent).toContain("hello?");

    // A second, fresher room appears — the activity sort now puts it first, and the agent starts
    // generating in it. The view must stay on the pinned room and pulse the busy one in the list.
    await render(
      root,
      [...beforeCommit, ...secondRoom],
      11,
      new Map([["conv-2", { ...generating, turnId: "T2" }]]),
    );
    expect(container.textContent).toContain("hello?");
    expect(container.textContent).not.toContain("over here!");
    const annexLink = [...container.querySelectorAll("button")].find(
      (candidate) => candidate.title === "annex",
    );
    expect(annexLink?.querySelector(".animate-ping")).toBeTruthy();

    act(() => root.unmount());
    container.remove();
  });
});

describe("the conversation view across the first step's commit", () => {
  it("keeps an opened deliberation open when the ModelCalled lands and the cursor advances", async () => {
    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);

    // Streaming: the pending turn's disclosure is opened by the reader.
    await render(root, beforeCommit, 3, new Map([["conv-1", generating]]));
    act(() => disclosureButton(container).click());
    expect(container.textContent).toContain("pondering the reply");

    // The step commits: the event lands, the cursor follows the head, and the supersede marks the
    // in-flight accumulation (the folds keep the object; only the agent's ConversationTurn or an
    // abandoned frame drops it — see lib/api/liveEval.test.ts for the fold-layer pin).
    await render(root, afterCommit, 4, new Map([["conv-1", { ...generating, superseded: true }]]));
    expect(container.textContent).toContain("pondered the reply");

    act(() => root.unmount());
    container.remove();
  });
});
