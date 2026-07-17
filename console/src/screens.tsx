import { lazy } from "react";

import type { LiveConnection } from "./lib/api/live.ts";
import type { LiveEvalStatus } from "./lib/api/liveEval.ts";
import type { AppLocation } from "./lib/nav/location.ts";
import { Redirect } from "./lib/nav/history.tsx";
import { useLocation } from "./lib/nav/historyContext.ts";
import { useAppStore } from "./lib/nav/appStore.ts";
import { Landing } from "./frames/landing/Landing.tsx";
import { EvalFrame } from "./frames/eval/EvalFrame.tsx";
import { RunFrame } from "./frames/eval/RunFrame.tsx";
import { ScenarioOverview } from "./views/ScenarioOverview.tsx";

// The agent frame (CodeMirror, the settings and prompts editors) and the trends screen (recharts) are
// heavy and reached from their own screens, so they load on demand.
const LiveShell = lazy(() =>
  import("./frames/live/LiveShell.tsx").then((module) => ({ default: module.LiveShell })),
);
const TrendsScreen = lazy(() =>
  import("./frames/trends/TrendsScreen.tsx").then((module) => ({ default: module.TrendsScreen })),
);

// The standing connection the embedded build holds: the agent it is served from, same origin, no key.
const AGENT: LiveConnection = { baseUrl: "", key: null };

const LANDING: AppLocation = { kind: "landing" };

/// The full console, rendered from the current location. This is the whole route table — a `switch`
/// over the typed [`AppLocation`] — in place of a route tree. A screen whose data is not loaded
/// recovers to the landing, since the data lives in memory, not at the URL, so a deep URL opened cold
/// lands somewhere coherent rather than blank.
export function ConsoleApp() {
  const location = useLocation();
  const store = useAppStore();

  switch (location.kind) {
    case "landing":
      return (
        <Landing
          onOpenPackage={store.openPackage}
          onOpenHistory={store.openHistory}
          onConnectLive={store.connectLive}
          onWatchEval={store.watchEval}
          error={store.error}
        />
      );
    case "trends":
      return store.history ? (
        <TrendsScreen entries={store.history} onClose={store.closeHistory} />
      ) : (
        <Redirect to={LANDING} />
      );
    case "evalOverview":
      return <EvalScreen run={false} />;
    case "stream":
      switch (location.frame.kind) {
        case "evalRun":
          return <EvalScreen run={true} />;
        case "live":
          return store.live ? (
            <LiveShell connection={store.live} onClose={store.closeLive} />
          ) : (
            <Redirect to={LANDING} />
          );
        case "embedded":
          // The embedded grammar never parses in console mode; recover defensively.
          return <Redirect to={LANDING} />;
      }
  }
}

/// The embedded console: the agent's live view is the whole app, connected to the agent it is served
/// from — no landing to return to, so no close affordance.
export function EmbeddedApp() {
  return <LiveShell connection={AGENT} />;
}

/// The eval frame around either the scenario overview or a single run's deep views, resolved from the
/// store: a watched live eval (its status screen until the first snapshot lands), a file-loaded
/// package, or — with neither — a redirect to the landing. The frame is the same either way; only its
/// inner screen differs.
function EvalScreen({ run }: { run: boolean }) {
  const store = useAppStore();
  const inner = run ? <RunFrame /> : <ScenarioOverview />;

  if (store.liveEvalConn) {
    return store.liveEval.pkg ? (
      <EvalFrame
        pkg={store.liveEval.pkg}
        live={store.liveEval.status}
        liveRuns={store.liveEval.liveRuns}
        progress={store.liveEval.progress}
        getRun={store.getLiveRun}
        onClose={store.closeLiveEval}
      >
        {inner}
      </EvalFrame>
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
    >
      {inner}
    </EvalFrame>
  ) : (
    <Redirect to={LANDING} />
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
