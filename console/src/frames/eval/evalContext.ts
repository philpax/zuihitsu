import { createContext, useContext } from "react";

import type { EvalContext } from "../../lib/api/liveEval.ts";

/// The open package and its live-run seams, provided by the eval frame to its inner screen (the
/// scenario overview or a run's deep views) so each resolves its scenario and run from the location
/// against it. The frame renders the screen as its children and wraps them in this context.
export const EvalRouteContext = createContext<EvalContext | null>(null);

export function useEvalContext(): EvalContext {
  const context = useContext(EvalRouteContext);
  if (!context) throw new Error("useEvalContext must be used within the eval frame");
  return context;
}
