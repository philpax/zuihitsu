import {
  useCallback,
  useEffect,
  useState,
  type AnchorHTMLAttributes,
  type MouseEvent,
  type ReactNode,
} from "react";

import { type AppLocation, type Mode, buildPath, parsePath } from "./location.ts";
import { LocationContext, NavigateContext, type Navigate, useNavigate } from "./historyContext.ts";

/// The browser-history binding for the [`AppLocation`] model: it owns the current location, keeps it
/// in sync with the address bar, and exposes navigating and linking. This is the entire router
/// runtime — a codec (`location.ts`) plus this thin history glue — in place of a route tree. The
/// read hooks (`useLocation`, `useNavigate`) live in `historyContext.ts`.

/// Owns the current location, parsed from the address bar and rewritten on navigation. A URL that
/// names no reachable place (a typo, a stale deep link) falls back to the mode's home rather than
/// stranding the app — the model's not-found story, without a route tree to configure.
export function RouterProvider({ mode, children }: { mode: Mode; children: ReactNode }) {
  const [location, setLocation] = useState<AppLocation>(
    () => parsePath(currentPath(), mode) ?? fallbackFor(mode),
  );

  useEffect(() => {
    // Back/forward changed the URL out from under us: re-parse the address bar into the model.
    const onPopState = () => setLocation(parsePath(currentPath(), mode) ?? fallbackFor(mode));
    window.addEventListener("popstate", onPopState);
    return () => window.removeEventListener("popstate", onPopState);
  }, [mode]);

  const navigate = useCallback<Navigate>((to, options) => {
    const path = buildPath(to);
    // The target is already the canonical location, so store it directly rather than re-parsing —
    // `parsePath(buildPath(to)) === to` by construction (see `location.test.ts`).
    if (options?.replace) window.history.replaceState(null, "", path);
    else window.history.pushState(null, "", path);
    setLocation(to);
  }, []);

  return (
    <NavigateContext.Provider value={navigate}>
      <LocationContext.Provider value={location}>{children}</LocationContext.Provider>
    </NavigateContext.Provider>
  );
}

/// A semantic client-side link to a typed location: a real `<a>` (so the browser's copy-link and
/// open-in-new-tab affordances work), whose plain left-click is intercepted for an in-app navigation.
/// Modified clicks (a new tab or window, a non-primary button) fall through to the browser.
export function Link({
  to,
  replace,
  onClick,
  children,
  ...rest
}: { to: AppLocation; replace?: boolean } & Omit<AnchorHTMLAttributes<HTMLAnchorElement>, "href">) {
  const navigate = useNavigate();
  const handleClick = (event: MouseEvent<HTMLAnchorElement>) => {
    onClick?.(event);
    if (event.defaultPrevented) return;
    if (event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) {
      return;
    }
    event.preventDefault();
    navigate(to, { replace });
  };
  return (
    <a href={buildPath(to)} onClick={handleClick} {...rest}>
      {children}
    </a>
  );
}

/// Navigate on mount, then render nothing — the model's redirect, for a screen whose data is not
/// loaded (recovering to the landing) or for an old URL rewritten to its canonical form. Replaces by
/// default so it leaves no trapping history entry.
export function Redirect({ to, replace = true }: { to: AppLocation; replace?: boolean }) {
  const navigate = useNavigate();
  useEffect(() => {
    navigate(to, { replace });
    // The target is a fresh literal each render; navigating once on mount is the intent.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  return null;
}

function currentPath(): string {
  return window.location.pathname + window.location.search;
}

function fallbackFor(mode: Mode): AppLocation {
  return mode === "embedded"
    ? { kind: "stream", frame: { kind: "embedded" }, stream: { view: "conversation", search: {} } }
    : { kind: "landing" };
}
