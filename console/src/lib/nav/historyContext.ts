import { createContext, useContext } from "react";

import type { AppLocation } from "./location.ts";

/// The router's contexts and read hooks, kept apart from the components in `history.tsx` so that file
/// exports only components (clean Fast Refresh boundaries). `RouterProvider` supplies these; every
/// other consumer reads them through the hooks.
export interface NavigateOptions {
  /// Rewrite the current entry instead of pushing a new one — for continuous interaction (the cursor
  /// scrubber, a filter toggle) and for recovering redirects, so the back button is not buried.
  replace?: boolean;
}

export type Navigate = (to: AppLocation, options?: NavigateOptions) => void;

export const LocationContext = createContext<AppLocation | null>(null);
export const NavigateContext = createContext<Navigate | null>(null);

export function useLocation(): AppLocation {
  const location = useContext(LocationContext);
  if (!location) throw new Error("useLocation must be used within the RouterProvider");
  return location;
}

export function useNavigate(): Navigate {
  const navigate = useContext(NavigateContext);
  if (!navigate) throw new Error("useNavigate must be used within the RouterProvider");
  return navigate;
}
