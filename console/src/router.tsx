import { createRootRoute, createRoute, createRouter } from "@tanstack/react-router";

import { RunFrame } from "./frames/eval/RunFrame.tsx";
import { ScenarioOverview } from "./views/ScenarioOverview.tsx";
import {
  EmbeddedLive,
  EmbeddedRootLayout,
  EvalRoute,
  LandingRoute,
  LiveIndexRoute,
  LiveRoute,
  NotFoundRedirect,
  RootLayout,
  TrendsRoute,
} from "./routeComponents.tsx";

// The URL's query state, shared by every route so the timeline cursor (`seq`), a turn highlight
// (`turn`), the Events pin (`focus`), and the Relations filters read and write through one typed
// schema. The stream views read it route-agnostically (they render under both the eval and live
// subtrees), so validation is lenient rather than per-route.
export interface StreamSearch {
  seq?: number;
  turn?: string;
  focus?: string;
  relations?: string;
  sameAs?: string;
  expand?: string;
}

function validateStreamSearch(search: Record<string, unknown>): StreamSearch {
  const out: StreamSearch = {};
  if (search.seq !== undefined && search.seq !== "") {
    const seq = Number(search.seq);
    if (!Number.isNaN(seq)) out.seq = seq;
  }
  for (const key of ["turn", "focus", "relations", "sameAs", "expand"] as const) {
    const value = search[key];
    if (typeof value === "string" && value !== "") out[key] = value;
  }
  return out;
}

// ---- The console route tree: the single source of truth for every route the full console serves. ----

const rootRoute = createRootRoute({
  component: RootLayout,
  validateSearch: validateStreamSearch,
  notFoundComponent: NotFoundRedirect,
});

const landingRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/",
  component: LandingRoute,
});

const evalRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "eval",
  component: EvalRoute,
});
const evalIndexRoute = createRoute({
  getParentRoute: () => evalRoute,
  path: "/",
  component: ScenarioOverview,
});
const runRoute = createRoute({
  getParentRoute: () => evalRoute,
  path: "$scenario/$run/$view",
  component: RunFrame,
});
const runSelectionRoute = createRoute({
  getParentRoute: () => evalRoute,
  path: "$scenario/$run/$view/$selection",
  component: RunFrame,
});

const trendsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "trends",
  component: TrendsRoute,
});

const liveIndexRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "live",
  component: LiveIndexRoute,
});
const liveViewRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "live/$view",
  component: LiveRoute,
});
const liveSelectionRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "live/$view/$selection",
  component: LiveRoute,
});

export const consoleRouter = createRouter({
  routeTree: rootRoute.addChildren([
    landingRoute,
    evalRoute.addChildren([evalIndexRoute, runRoute, runSelectionRoute]),
    trendsRoute,
    liveIndexRoute,
    liveViewRoute,
    liveSelectionRoute,
  ]),
});

// ---- The embedded route tree: the agent's live view is the whole app, served at the root. ----

const embeddedRootRoute = createRootRoute({
  component: EmbeddedRootLayout,
  validateSearch: validateStreamSearch,
  notFoundComponent: NotFoundRedirect,
});
const embeddedIndexRoute = createRoute({
  getParentRoute: () => embeddedRootRoute,
  path: "/",
  component: EmbeddedLive,
});
const embeddedViewRoute = createRoute({
  getParentRoute: () => embeddedRootRoute,
  path: "$view",
  component: EmbeddedLive,
});
const embeddedSelectionRoute = createRoute({
  getParentRoute: () => embeddedRootRoute,
  path: "$view/$selection",
  component: EmbeddedLive,
});

export const embeddedRouter = createRouter({
  routeTree: embeddedRootRoute.addChildren([
    embeddedIndexRoute,
    embeddedViewRoute,
    embeddedSelectionRoute,
  ]),
});

// The full console is the app the typed hooks resolve against; the embedded build shares the same
// route-agnostic stream views and reaches its few destinations by string (and by `strict: false`
// reads), so one registration serves both. The seam to respect: a *typed* `to`/`params` written in a
// shared view would typecheck against this console tree yet run against `embeddedRouter`'s different
// tree at runtime — so shared-view navigation stays string-built (see `routes.ts`), never a typed
// route literal, which would only be sound in a console-only component.
declare module "@tanstack/react-router" {
  interface Register {
    router: typeof consoleRouter;
  }
}
