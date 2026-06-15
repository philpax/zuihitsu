import { lazy, Suspense, useState } from "react";
import { BrowserRouter, Navigate, Route, Routes, useNavigate } from "react-router-dom";

import type { EvalPackage } from "./types/EvalPackage.ts";
import type { LiveConnection } from "./lib/live.ts";
import { type HistoryEntry, parseHistory } from "./lib/history.ts";
import { loadPackageFromFile } from "./lib/package.ts";
import { ConsoleNavContext } from "./lib/consoleNav.ts";
import { Landing } from "./components/Landing.tsx";
import { EvalFrame } from "./components/EvalFrame.tsx";
import { RunFrame } from "./components/RunFrame.tsx";
import { ScenarioOverview } from "./views/ScenarioOverview.tsx";

// The agent frame (CodeMirror, the settings and prompts editors) and the trends screen (recharts)
// are heavy and reached from their own routes, so they load on demand rather than weighing down the
// eval viewer's first paint.
const LiveShell = lazy(() =>
  import("./components/LiveShell.tsx").then((module) => ({ default: module.LiveShell })),
);
const TrendsScreen = lazy(() =>
  import("./components/TrendsScreen.tsx").then((module) => ({ default: module.TrendsScreen })),
);

export function App() {
  return (
    <BrowserRouter>
      <Console />
    </BrowserRouter>
  );
}

/// The console proper, inside the router so its loaders can navigate. It holds the package, the
/// metrics history, and a live connection *independently* — they are the same eval seen different
/// ways, not mutually exclusive modes — and exposes the loaders through context so any frame can pivot
/// to a sibling section without returning to the landing. A route whose data is not loaded redirects
/// to the landing, since the data lives in memory, not at the URL — so a deep URL opened cold lands
/// somewhere coherent rather than blank.
function Console() {
  const navigate = useNavigate();
  const [pkg, setPkg] = useState<EvalPackage | null>(null);
  const [history, setHistory] = useState<HistoryEntry[] | null>(null);
  const [live, setLive] = useState<LiveConnection | null>(null);
  const [error, setError] = useState<string | null>(null);
  // The file being read, so the wait on a large package shows feedback rather than a frozen page.
  const [reading, setReading] = useState<string | null>(null);

  async function openPackage(file: File) {
    setReading(file.name);
    try {
      setPkg(await loadPackageFromFile(file));
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

  function connectLive() {
    setLive({ baseUrl: "", key: null });
    setError(null);
    navigate("/live");
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
                error={error}
              />
            }
          />

          <Route
            path="/eval"
            element={
              pkg ? (
                <EvalFrame pkg={pkg} onClose={() => setPkg(null)} />
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
