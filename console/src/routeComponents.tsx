import { lazy, Suspense } from "react";
import { Navigate, Outlet } from "@tanstack/react-router";

import type { LiveConnection } from "./lib/api/live.ts";
import type { LiveEvalStatus } from "./lib/api/liveEval.ts";
import { useAppStore } from "./lib/nav/appStore.ts";
import { Landing } from "./frames/landing/Landing.tsx";
import { EvalFrame } from "./frames/eval/EvalFrame.tsx";

// The agent frame (CodeMirror, the settings and prompts editors) and the trends screen (recharts) are
// heavy and reached from their own routes, so they load on demand rather than weighing down the eval
// viewer's first paint.
const LiveShell = lazy(() =>
  import("./frames/live/LiveShell.tsx").then((module) => ({ default: module.LiveShell })),
);
const TrendsScreen = lazy(() =>
  import("./frames/trends/TrendsScreen.tsx").then((module) => ({ default: module.TrendsScreen })),
);

// The standing connection the embedded build holds: the agent it is served from, same origin, no key
// (a loopback peer is trusted; see the server's auth).
const AGENT: LiveConnection = { baseUrl: "", key: null };

/// The console's root layout: a Suspense boundary over the matched route, so a lazily-loaded route
/// chunk shows the calm loading screen while it resolves.
export function RootLayout() {
  return (
    <Suspense fallback={<LoadingScreen label="Loading…" />}>
      <Outlet />
    </Suspense>
  );
}

/// The embedded build's root layout — the agent's live view is the whole app, so the fallback speaks
/// to connecting rather than loading.
export function EmbeddedRootLayout() {
  return (
    <Suspense fallback={<LoadingScreen label="Connecting to the agent…" />}>
      <Outlet />
    </Suspense>
  );
}

/// The landing: opens a package, a history file, or a live connection — the store's actions.
export function LandingRoute() {
  const store = useAppStore();
  return (
    <Landing
      onOpenPackage={store.openPackage}
      onOpenHistory={store.openHistory}
      onConnectLive={store.connectLive}
      onWatchEval={store.watchEval}
      error={store.error}
    />
  );
}

/// The eval frame, resolved from the store: a watched live eval (its status screen until the first
/// snapshot lands), a file-loaded package, or — with neither loaded — a redirect to the landing, since
/// the data lives in memory, not at the URL.
export function EvalRoute() {
  const store = useAppStore();
  if (store.liveEvalConn) {
    return store.liveEval.pkg ? (
      <EvalFrame
        pkg={store.liveEval.pkg}
        live={store.liveEval.status}
        liveRuns={store.liveEval.liveRuns}
        progress={store.liveEval.progress}
        getRun={store.getLiveRun}
        onClose={store.closeLiveEval}
      />
    ) : (
      <LiveEvalStatusScreen status={store.liveEval.status} onClose={store.closeLiveEval} />
    );
  }
  return store.fileSummary ? (
    <EvalFrame
      pkg={store.fileSummary}
      fileName={store.fileName}
      getRun={store.getFileRun}
      onClose={store.closePackage}
    />
  ) : (
    <Navigate to="/" replace />
  );
}

export function TrendsRoute() {
  const store = useAppStore();
  return store.history ? (
    <TrendsScreen entries={store.history} onClose={store.closeHistory} />
  ) : (
    <Navigate to="/" replace />
  );
}

export function LiveIndexRoute() {
  const store = useAppStore();
  return store.live ? (
    <Navigate to="/live/$view" params={{ view: "conversation" }} replace />
  ) : (
    <Navigate to="/" replace />
  );
}

export function LiveRoute() {
  const store = useAppStore();
  return store.live ? (
    <LiveShell connection={store.live} onClose={store.closeLive} />
  ) : (
    <Navigate to="/" replace />
  );
}

/// The not-found fallback for both route trees, standing in for react-router's old catch-all `*`
/// route: a URL matching no route (a typo, a shared link gone stale, a partial `/eval/:scenario` with
/// no run) recovers to the root rather than stranding on a bare "Not Found" — and replaces, so it
/// leaves no trapping history entry.
export function NotFoundRedirect() {
  return <Navigate to="/" replace />;
}

/// The embedded console's live view, connected to the agent it is served from — no landing to return
/// to, so no close affordance.
export function EmbeddedLive() {
  return <LiveShell connection={AGENT} base="" />;
}

/// A calm, centered status while a lazily-loaded route chunk resolves.
function LoadingScreen({ label }: { label: string }) {
  return (
    <div className="flex min-h-screen items-center justify-center text-sm text-ink-faint">
      {label}
    </div>
  );
}

/// Shown while a live eval is connecting (before its first snapshot) or stalled — once the snapshot
/// lands, the eval frame takes over. The close returns to the landing.
function LiveEvalStatusScreen({
  status,
  onClose,
}: {
  status: LiveEvalStatus;
  onClose: () => void;
}) {
  const message = status.status === "error" ? status.message : "Connecting to the live eval…";
  return (
    <div className="flex min-h-screen flex-col items-center justify-center gap-4">
      <p className={"text-sm " + (status.status === "error" ? "text-clay" : "text-ink-faint")}>
        {message}
      </p>
      <button
        onClick={onClose}
        className="font-mono text-xs text-ink-faint transition-colors hover:text-clay"
      >
        ← back
      </button>
    </div>
  );
}
