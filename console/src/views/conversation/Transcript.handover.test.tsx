// @vitest-environment jsdom
import { beforeAll, describe, expect, it, vi } from "vitest";
import { act } from "react";
import { StrictMode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { MemoryRouter } from "react-router-dom";

import { emptyTurn, type ConversationModel } from "../../lib/model/conversation.ts";
import type { InFlightGeneration } from "../../lib/model/inflight.ts";
import type { Replica } from "../../lib/replica/replica.ts";
import { Transcript } from "./Transcript.tsx";

// The wasm bridge needs a browser fetch to initialise; under jsdom the ref scanner is stubbed to
// "no references", which every fixture text here satisfies.
vi.mock("../../lib/replica/replica.ts", async (importOriginal) => ({
  ...(await importOriginal<object>()),
  scanTurnRefs: (text: string) => [{ kind: "prose", text }],
}));

// jsdom lacks the browser APIs motion/react feature-detects; stub them before anything renders.
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

function conversation(
  turns: ConversationModel["turns"],
  sessions: ConversationModel["sessions"] = [],
): ConversationModel {
  return {
    id: "conv-1",
    platform: "test",
    scopePath: "room",
    contextName: null,
    contextMemory: null,
    sessions,
    turns,
  };
}

/// A session opened at the log's start, so the transcript takes its sessions branch — the shape
/// every live conversation has.
function openingSession(): ConversationModel["sessions"][number] {
  return {
    id: "session-1",
    seq: 1,
    brief: "the room's brief",
    startedAt: 1_000,
    participants: ["someone"],
    participantIds: ["person/someone"],
    compaction: false,
    workingSet: null,
  };
}

const generating: InFlightGeneration = {
  turnId: "T1",
  step: 0,
  phase: "Step",
  reasoning: "pondering the reply",
  reply: "",
  restarts: 0,
  superseded: false,
};

/// The turn as the fold materialises it at its first committed deliberation event: `emptyTurn`
/// completed by one recorded model step, `ConversationTurn` still to come.
function materialisedTurn(): ConversationModel["turns"][number] {
  const turn = emptyTurn("T1", 5);
  turn.deliberation.push({
    kind: "model",
    seq: 5,
    phase: "Step",
    reasoning: "pondered the reply",
    completion: { Reply: "hello" },
    finishReason: null,
    durationMs: 1200,
  });
  return turn;
}

function render(
  root: Root,
  inflight: InFlightGeneration | null,
  turns: ConversationModel["turns"],
  sessions: ConversationModel["sessions"] = [],
) {
  act(() => {
    root.render(
      <StrictMode>
        <MemoryRouter>
          <Transcript
            replica={{} as Replica}
            conversation={conversation(turns, sessions)}
            cursor={0}
            inflight={inflight}
          />
        </MemoryRouter>
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

describe("the pending → materialised turn handover", () => {
  it("keeps an opened deliberation open across the first step's commit", () => {
    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);

    // A turn that exists only as streamed tokens: the pending item holds the tail slot.
    render(root, generating, []);
    const button = disclosureButton(container);
    expect(button.textContent).toContain("generating…");
    act(() => button.click());
    expect(container.textContent).toContain("pondering the reply");

    // The first step commits: the fold materialises the turn and the supersede clears the
    // in-flight accumulation, in one update. The opened disclosure must ride through.
    render(root, null, [materialisedTurn()]);
    expect(container.textContent).toContain("pondered the reply");

    act(() => root.unmount());
    container.remove();
  });

  it("streams the reply into the message body, visible without opening the deliberation", () => {
    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);

    // The disclosure stays closed: the thinking is opt-in, the speech is not.
    render(root, { ...generating, reply: "Hello wor" }, []);
    expect(container.textContent).toContain("Hello wor");
    expect(container.textContent).not.toContain("pondering the reply");
    // Once the step commits (marked superseded), the streamed text yields until the fold's own
    // text takes over — never two copies at once.
    render(root, { ...generating, reply: "Hello wor", superseded: true }, []);
    expect(container.textContent).not.toContain("Hello wor");

    act(() => root.unmount());
    container.remove();
  });

  it("keeps the disclosure open across the next step streaming in", () => {
    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);

    render(root, generating, []);
    act(() => disclosureButton(container).click());
    render(root, null, [materialisedTurn()]);
    // Step 2 begins streaming into the now-materialised turn.
    render(root, { ...generating, step: 1, reasoning: "second thoughts" }, [materialisedTurn()]);
    expect(container.textContent).toContain("pondered the reply");
    expect(container.textContent).toContain("second thoughts");

    act(() => root.unmount());
    container.remove();
  });

  it("keeps an opened deliberation open across the handover in the sessions branch", () => {
    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);

    // The shape a live conversation actually has: the session opened, the pending item held in the
    // last session's list, and the materialised turn landing in that same list.
    render(root, generating, [], [openingSession()]);
    act(() => disclosureButton(container).click());
    expect(container.textContent).toContain("pondering the reply");
    render(root, null, [materialisedTurn()], [openingSession()]);
    expect(container.textContent).toContain("pondered the reply");

    act(() => root.unmount());
    container.remove();
  });
});
