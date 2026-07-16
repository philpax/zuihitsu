// @vitest-environment jsdom
import { beforeAll, describe, expect, it } from "vitest";
import { act, StrictMode } from "react";
import { createRoot, type Root } from "react-dom/client";
import {
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
  RouterProvider,
} from "@tanstack/react-router";

import { AppStoreProvider } from "./lib/nav/AppStoreProvider.tsx";
import { EvalRoute, LiveIndexRoute, NotFoundRedirect } from "./routeComponents.tsx";
import { useStreamLocation } from "./lib/nav/useStreamLocation.ts";
import { consoleRouter, embeddedRouter } from "./router.tsx";

// Routing integration: a real `RouterProvider` over the real store-gated route components, exercising
// the redirect and search behaviour the handover tests deliberately mock away. The store starts empty
// (no package, no live connection), so the data-gated routes take their redirect branch.

beforeAll(() => {
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
});

/// A scrubber standing in for a stream view, so a click drives the real `setSeq` through the router.
function Scrubber() {
  const { seq, setSeq } = useStreamLocation("/live");
  return <button onClick={() => setSeq(5)}>{seq === null ? "head" : `seq ${seq}`}</button>;
}

const rootRoute = createRootRoute({ notFoundComponent: NotFoundRedirect });
const landingRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/",
  component: () => <div>landing</div>,
});
const evalRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "eval",
  component: EvalRoute,
});
const liveIndexRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "live",
  component: LiveIndexRoute,
});
const liveViewRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "live/$view",
  component: Scrubber,
});
const routeTree = rootRoute.addChildren([landingRoute, evalRoute, liveIndexRoute, liveViewRoute]);

function makeRouter(initial: string) {
  return createRouter({
    routeTree,
    history: createMemoryHistory({ initialEntries: [initial] }),
  });
}

async function mount(root: Root, router: ReturnType<typeof makeRouter>) {
  await act(async () => {
    root.render(
      <StrictMode>
        <AppStoreProvider navigate={() => {}}>
          <RouterProvider router={router} />
        </AppStoreProvider>
      </StrictMode>,
    );
  });
}

function withRoot(body: (root: Root, container: HTMLElement) => Promise<void>) {
  return async () => {
    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);
    try {
      await body(root, container);
    } finally {
      act(() => root.unmount());
      container.remove();
    }
  };
}

describe("the route tree's redirects", () => {
  it(
    "sends a data-less deep route back to the landing, and replaces rather than pushes",
    withRoot(async (root) => {
      const router = makeRouter("/eval");
      await mount(root, router);

      // The empty store has no package, so `/eval` recovers to the landing.
      expect(router.state.location.pathname).toBe("/");
      // Replace, not push: the single memory entry was rewritten, so Back does not bounce to `/eval`.
      expect(router.history.length).toBe(1);
    }),
  );

  it(
    "recovers an unknown URL to the landing instead of a bare Not Found",
    withRoot(async (root, container) => {
      const router = makeRouter("/eval/scenario-with-no-run-or-view");
      await mount(root, router);

      expect(router.state.location.pathname).toBe("/");
      expect(container.textContent).toContain("landing");
      expect(container.textContent).not.toContain("Not Found");
    }),
  );

  // The recovery above rides a local tree; guard that both real routers actually wire the handler, so
  // a future edit dropping it (back to TanStack's bare "Not Found") is caught here rather than in use.
  it("wires a not-found handler on both real route trees", () => {
    expect(consoleRouter.routeTree.options.notFoundComponent).toBeTypeOf("function");
    expect(embeddedRouter.routeTree.options.notFoundComponent).toBeTypeOf("function");
  });
});

describe("the timeline cursor round-trips through search", () => {
  it(
    "pins ?seq while leaving the view path intact",
    withRoot(async (root, container) => {
      const router = makeRouter("/live/conversation");
      await mount(root, router);
      expect(container.textContent).toContain("head");

      await act(async () => {
        container.querySelector("button")!.click();
      });

      expect(router.state.location.pathname).toBe("/live/conversation");
      expect(router.state.location.search).toMatchObject({ seq: 5 });
      expect(container.textContent).toContain("seq 5");
    }),
  );
});
