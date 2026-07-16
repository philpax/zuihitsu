import { createContext, useContext } from "react";

import type { EvalContext } from "../../lib/api/liveEval.ts";

/// The open package and its live-run seams, provided by the eval frame to its nested routes (the
/// scenario overview and a run's deep views) so each resolves its scenario and run from the URL
/// against it. This stands in for react-router's `<Outlet context>`: TanStack renders nested routes
/// through a bare `<Outlet />`, so the frame passes shared data down through a React context instead.
export const EvalRouteContext = createContext<EvalContext | null>(null);

export function useEvalContext(): EvalContext {
  const context = useContext(EvalRouteContext);
  if (!context) throw new Error("useEvalContext must be used within the eval frame");
  return context;
}
