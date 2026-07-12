import { lazy, Suspense, useEffect, useRef, useState } from "react";
import { BrowserRouter, Navigate, Route, Routes, useNavigate } from "react-router-dom";

import type { EvalPackage } from "./types/EvalPackage.ts";
import type { LiveConnection } from "./lib/api/live.ts";
import { type LiveEvalConnection, type LiveEvalStatus, useLiveEval } from "./lib/api/liveEval.ts";
import { type HistoryEntry, parseHistory } from "./lib/model/history.ts";
import { loadPackageFromFile } from "./lib/replica/package.ts";
import { ConsoleNavContext } from "./lib/nav/consoleNav.ts";
import { Landing } from "./frames/landing/Landing.tsx";
import { EvalFrame } from "./frames/eval/EvalFrame.tsx";
import { RunFrame } from "./frames/eval/RunFrame.tsx";
import { ScenarioOverview } from "./views/ScenarioOverview.tsx";

// The agent frame (CodeMirror, the settings and prompts editors) and the trends screen (recharts)
// are heavy and reached from their own routes, so they load on demand rather than weighing down the
// eval viewer's first paint.
const LiveShell = lazy(() =>
  import("./frames/live/LiveShell.tsx").then((module) => ({ default: module.LiveShell })),
);
const TrendsScreen = lazy(() =>
  import("./frames/trends/TrendsScreen.tsx").then((module) => ({ default: module.TrendsScreen })),
);

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

// The standing connection the embedded build holds: the agent it is served from, same origin, no key
// (a loopback peer is trusted; see the server's auth).
const AGENT: LiveConnection = { baseUrl: "", key: null };

export function App() {
  return (
    <BrowserRouter>
      {MODE === "agent" ? <EmbeddedConsole /> : <Console autoWatchEval={MODE === "eval"} />}
    </BrowserRouter>
  );
}

/// The embedded console: the agent's live view, served at the agent's root and connected to it
/// automatically. The live views are the whole app, so they sit at the root with no landing to return
/// to (and so no close affordance).
function EmbeddedConsole() {
  return (
    <Suspense fallback={<LoadingScreen label="Connecting to the agent…" />}>
      <Routes>
        <Route path="/:view" element={<LiveShell connection={AGENT} base="" />} />
        <Route path="*" element={<Navigate to="/conversation" replace />} />
      </Routes>
    </Suspense>
  );
}

/// The console proper, inside the router so its loaders can navigate. It holds the package, the
/// metrics history, and a live connection *independently* — they are the same eval seen different
/// ways, not mutually exclusive modes — and exposes the loaders through context so any frame can pivot
/// to a sibling section without returning to the landing. A route whose data is not loaded redirects
/// to the landing, since the data lives in memory, not at the URL — so a deep URL opened cold lands
/// somewhere coherent rather than blank.
function Console({ autoWatchEval = false }: { autoWatchEval?: boolean }) {
  const navigate = useNavigate();
  const [pkg, setPkg] = useState<EvalPackage | null>(null);
  // The package's file name, kept for the header — the package itself does not carry it.
  const [pkgName, setPkgName] = useState<string | null>(null);
  const [history, setHistory] = useState<HistoryEntry[] | null>(null);
  const [live, setLive] = useState<LiveConnection | null>(null);
  // Watching a live eval: the harness's address, folded into a growing package by the hook below. In
  // eval mode the console is served by the eval binary itself, so it starts already pointed at the
  // same-origin live eval.
  const [liveEvalConn, setLiveEvalConn] = useState<LiveEvalConnection | null>(
    autoWatchEval ? { baseUrl: "" } : null,
  );
  const liveEval = useLiveEval(liveEvalConn);
  const [error, setError] = useState<string | null>(null);
  // The file being read, so the wait on a large package shows feedback rather than a frozen page.
  const [reading, setReading] = useState<string | null>(null);

  // Open the live eval as the initial view in eval mode — once. `navigate`'s identity changes on every
  // location change, so without this one-shot guard the effect would re-fire on each in-app navigation
  // and bounce the user back to /eval (e.g. clicking into a scenario would snap straight back). The
  // landing stays reachable at "/" for opening a past package to compare — only the first screen
  // changes.
  const openedInitialView = useRef(false);
  useEffect(() => {
    if (openedInitialView.current || !autoWatchEval) return;
    openedInitialView.current = true;
    navigate("/eval");
  }, [autoWatchEval, navigate]);

  async function openPackage(file: File) {
    setReading(file.name);
    try {
      setPkg(await loadPackageFromFile(file));
      setPkgName(file.name);
      setError(null);
      setReading(null);
      navigate("/eval");
    } catch (cause) {
      setError(describe(file, cause));
      setReading(null);
    }
  }

  async function openHistory(file: File) {
    setReading(file.name);
    try {
      setHistory(parseHistory(await file.text()));
      setError(null);
      setReading(null);
      navigate("/trends");
    } catch (cause) {
      setError(describe(file, cause));
      setReading(null);
    }
  }

  function connectLive(baseUrl: string) {
    // Same origin (the default) talks to the dev proxy or the embedding agent; a trailing slash is
    // trimmed so `${baseUrl}/control/...` is well-formed. A loopback peer needs no key.
    setLive({ baseUrl: baseUrl.trim().replace(/\/$/, ""), key: null });
    setError(null);
    navigate("/live");
  }

  function watchEval(baseUrl: string) {
    // Trim a trailing slash so `${baseUrl}/eval/stream` is well-formed whichever way it was typed.
    setLiveEvalConn({ baseUrl: baseUrl.trim().replace(/\/$/, "") });
    setError(null);
    navigate("/eval");
  }

  const nav = {
    hasPackage: pkg !== null,
    hasHistory: history !== null,
    openPackage,
    openHistory,
  };

  if (reading) return <LoadingScreen label={`Reading ${reading}…`} />;

  return (
    <ConsoleNavContext.Provider value={nav}>
      <Suspense fallback={<LoadingScreen label="Loading…" />}>
        <Routes>
          <Route
            path="/"
            element={
              <Landing
                onOpenPackage={openPackage}
                onOpenHistory={openHistory}
                onConnectLive={connectLive}
                onWatchEval={watchEval}
                error={error}
              />
            }
          />

          <Route
            path="/eval"
            element={
              liveEvalConn ? (
                liveEval.pkg ? (
                  <EvalFrame
                    pkg={liveEval.pkg}
                    live={liveEval.status}
                    liveRuns={liveEval.liveRuns}
                    progress={liveEval.progress}
                    onClose={() => setLiveEvalConn(null)}
                  />
                ) : (
                  <LiveEvalStatusScreen
                    status={liveEval.status}
                    onClose={() => setLiveEvalConn(null)}
                  />
                )
              ) : pkg ? (
                <EvalFrame
                  pkg={pkg}
                  fileName={pkgName}
                  onClose={() => {
                    setPkg(null);
                    setPkgName(null);
                  }}
                />
              ) : (
                <Navigate to="/" replace />
              )
            }
          >
            <Route index element={<ScenarioOverview />} />
            <Route path=":scenario/:run/:view" element={<RunFrame />} />
          </Route>

          <Route
            path="/trends"
            element={
              history ? (
                <TrendsScreen entries={history} onClose={() => setHistory(null)} />
              ) : (
                <Navigate to="/" replace />
              )
            }
          />

          <Route
            path="/live"
            element={
              live ? <Navigate to="/live/conversation" replace /> : <Navigate to="/" replace />
            }
          />
          <Route
            path="/live/:view"
            element={
              live ? (
                <LiveShell connection={live} onClose={() => setLive(null)} />
              ) : (
                <Navigate to="/" replace />
              )
            }
          />

          <Route path="*" element={<Navigate to="/" replace />} />
        </Routes>
      </Suspense>
    </ConsoleNavContext.Provider>
  );
}

/// A calm, centered status while something loads — a lazily-loaded route chunk, or a package being
/// read and parsed (a soak package is tens of megabytes, so the parse is not instant).
function LoadingScreen({ label }: { label: string }) {
  return (
    <div className="flex min-h-screen items-center justify-center text-sm text-ink-faint">
      {label}
    </div>
  );
}

function describe(file: File, cause: unknown): string {
  const message = cause instanceof Error ? cause.message : String(cause);
  return `Could not read ${file.name} — ${message}`;
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
