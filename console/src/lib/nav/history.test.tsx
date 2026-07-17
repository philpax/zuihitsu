// @vitest-environment jsdom
import { beforeAll, beforeEach, describe, expect, it } from "vitest";
import { act, StrictMode, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";

import type { AppLocation } from "./location.ts";
import { Link, Redirect, RouterProvider } from "./history.tsx";
import { useLocation, useNavigate } from "./historyContext.ts";
import { useStream } from "./useStreamLocation.ts";

// The router runtime is a codec (exhaustively tested in `location.test.ts`) plus this thin history
// glue. Here we drive the glue through a real DOM: the address bar stays in sync, push and replace
// behave, links navigate on a plain click, and a stream view round-trips its cursor.

beforeAll(() => {
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
});

beforeEach(() => {
  window.history.replaceState(null, "", "/");
});

function withRoot(path: string, tree: ReactNode, body: (container: HTMLElement) => void) {
  return () => {
    window.history.replaceState(null, "", path);
    const container = document.createElement("div");
    document.body.appendChild(container);
    const root: Root = createRoot(container);
    try {
      act(() => root.render(<StrictMode>{tree}</StrictMode>));
      body(container);
    } finally {
      act(() => root.unmount());
      container.remove();
    }
  };
}

/// Surfaces the current location's kind, and drives navigation from clicks.
function Probe() {
  const location = useLocation();
  const navigate = useNavigate();
  return (
    <div>
      <span data-testid="kind">{location.kind}</span>
      <button data-testid="push" onClick={() => navigate({ kind: "trends" })}>
        push
      </button>
      <button
        data-testid="replace"
        onClick={() => navigate({ kind: "landing" }, { replace: true })}
      >
        replace
      </button>
    </div>
  );
}

/// A stream view standing in for the real ones: shows and pins the cursor through `useStream`.
function Scrubber() {
  const { seq, setSeq } = useStream();
  return (
    <button data-testid="seq" onClick={() => setSeq(5)}>
      {seq === null ? "head" : `seq ${seq}`}
    </button>
  );
}

const kind = (c: HTMLElement) => c.querySelector('[data-testid="kind"]')!.textContent;

describe("the router runtime", () => {
  it(
    "parses the initial address bar into a typed location",
    withRoot(
      "/eval",
      <RouterProvider mode="console">
        <Probe />
      </RouterProvider>,
      (container) => expect(kind(container)).toBe("evalOverview"),
    ),
  );

  it(
    "falls back to the landing when the URL names no place",
    withRoot(
      "/nonsense/deep",
      <RouterProvider mode="console">
        <Probe />
      </RouterProvider>,
      (container) => expect(kind(container)).toBe("landing"),
    ),
  );

  it(
    "pushes a new entry on navigate, and rewrites in place on replace",
    withRoot(
      "/",
      <RouterProvider mode="console">
        <Probe />
      </RouterProvider>,
      (container) => {
        const before = window.history.length;
        act(() => container.querySelector<HTMLButtonElement>('[data-testid="push"]')!.click());
        expect(window.location.pathname).toBe("/trends");
        expect(kind(container)).toBe("trends");
        expect(window.history.length).toBe(before + 1);

        // Replace: the entry is rewritten, so the history depth does not grow — no trapping entry.
        const afterPush = window.history.length;
        act(() => container.querySelector<HTMLButtonElement>('[data-testid="replace"]')!.click());
        expect(window.location.pathname).toBe("/");
        expect(window.history.length).toBe(afterPush);
      },
    ),
  );

  it(
    "redirects (replacing) a screen whose place is a dead end",
    withRoot(
      "/trends",
      <RouterProvider mode="console">
        <Redirect to={{ kind: "landing" } satisfies AppLocation} />
      </RouterProvider>,
      () => {
        expect(window.location.pathname).toBe("/");
      },
    ),
  );

  it(
    "renders a link's canonical href and navigates on a plain click",
    withRoot(
      "/",
      <RouterProvider mode="console">
        <Link to={{ kind: "trends" }} data-testid="link">
          trends
        </Link>
      </RouterProvider>,
      (container) => {
        const link = container.querySelector<HTMLAnchorElement>('[data-testid="link"]')!;
        expect(new URL(link.href).pathname).toBe("/trends");
        act(() => link.click());
        expect(window.location.pathname).toBe("/trends");
      },
    ),
  );

  it(
    "pins the cursor into the search while leaving the view path intact",
    withRoot(
      "/live/state/self",
      <RouterProvider mode="console">
        <Scrubber />
      </RouterProvider>,
      (container) => {
        const button = container.querySelector<HTMLButtonElement>('[data-testid="seq"]')!;
        expect(button.textContent).toBe("head");
        act(() => button.click());
        expect(window.location.pathname).toBe("/live/state/self");
        expect(window.location.search).toBe("?seq=5");
        expect(button.textContent).toBe("seq 5");
      },
    ),
  );
});
