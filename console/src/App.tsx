import { Suspense } from "react";

import { RouterProvider } from "./lib/nav/history.tsx";
import { AppStoreProvider } from "./lib/nav/AppStoreProvider.tsx";
import { ConsoleApp, EmbeddedApp } from "./screens.tsx";

// The serving binary announces its mode at runtime via `window.__APP_MODE__` (the template token in
// index.html is replaced at serve time), so one built bundle serves every host. The agent serves its
// own focused live view; the eval binary serves the console pointed at its same-origin live eval;
// anything else (a standalone build with the token left unreplaced) is the full console.
declare global {
  interface Window {
    __APP_MODE__?: string;
  }
}
type AppMode = "agent" | "eval" | "console";
const MODE: AppMode =
  window.__APP_MODE__ === "agent" || window.__APP_MODE__ === "eval"
    ? window.__APP_MODE__
    : "console";

export function App() {
  // The embedded build (the agent's own live view) is the whole app, served at the root under the
  // embedded URL grammar — no landing, so no store to hold a package.
  if (MODE === "agent") {
    return (
      <RouterProvider mode="embedded">
        <Suspense fallback={<Loading label="Connecting to the agent…" />}>
          <EmbeddedApp />
        </Suspense>
      </RouterProvider>
    );
  }

  // The full console holds the package, the metrics history, and a live connection in the store; the
  // screens render from it and from the current location.
  return (
    <RouterProvider mode="console">
      <AppStoreProvider autoWatchEval={MODE === "eval"}>
        <Suspense fallback={<Loading label="Loading…" />}>
          <ConsoleApp />
        </Suspense>
      </AppStoreProvider>
    </RouterProvider>
  );
}

function Loading({ label }: { label: string }) {
  return (
    <div className="flex min-h-screen items-center justify-center text-sm text-ink-faint">
      {label}
    </div>
  );
}
