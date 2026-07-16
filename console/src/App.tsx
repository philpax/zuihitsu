import { RouterProvider } from "@tanstack/react-router";

import { AppStoreProvider } from "./lib/nav/AppStoreProvider.tsx";
import { consoleRouter, embeddedRouter } from "./router.tsx";

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
  // The embedded build (the agent's own live view) is the whole app, served at the root with its own
  // route tree and connected to the agent automatically — no landing, so no store to hold a package.
  if (MODE === "agent") return <RouterProvider router={embeddedRouter} />;

  // The full console holds the package, the metrics history, and a live connection in the store; the
  // router renders from it. The store's actions navigate through the router instance directly, since
  // the store provider sits outside the router's own hook context.
  return (
    <AppStoreProvider
      autoWatchEval={MODE === "eval"}
      navigate={(to) => consoleRouter.navigate({ to })}
    >
      <RouterProvider router={consoleRouter} />
    </AppStoreProvider>
  );
}
