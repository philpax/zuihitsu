import { lazy, Suspense, useState } from "react";
import { BrowserRouter, Navigate, Route, Routes, useNavigate } from "react-router-dom";

import type { EvalPackage } from "./types/EvalPackage.ts";
import type { LiveConnection } from "./lib/live.ts";
import { type HistoryEntry, parseHistory } from "./lib/history.ts";
import { loadPackageFromFile } from "./lib/package.ts";
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

/// What the console has open: an eval package (one run suite, the deep views), the metrics history
/// (trends over time), or a live connection to a running agent (the same deep views, tailed). The
/// router reflects *where in* the open thing you are — which scenario, run, view, and timeline cursor
/// — so the browser's back and forward move through the inspection, not merely in and out of it.
type Loaded =
  | { kind: "package"; pkg: EvalPackage }
  | { kind: "history"; entries: HistoryEntry[] }
  | { kind: "live"; connection: LiveConnection };

/// The root: hold what is open, and route between the empty state, the package frame (with its nested
/// scenario and run routes), the trends screen, and the live agent frame. A route whose data is not
/// loaded redirects to the landing, so a deep URL opened cold — the package lives in memory, not at
/// the URL — lands somewhere coherent rather than blank.
export function App() {
  const [loaded, setLoaded] = useState<Loaded | null>(null);

  return (
    <BrowserRouter>
      <Suspense fallback={<LoadingScreen label="Loading…" />}>
        <Routes>
          <Route path="/" element={<LandingRoute setLoaded={setLoaded} />} />

          <Route
            path="/eval"
            element={
              loaded?.kind === "package" ? (
                <EvalFrame pkg={loaded.pkg} onClose={() => setLoaded(null)} />
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
              loaded?.kind === "history" ? (
                <TrendsScreen entries={loaded.entries} onClose={() => setLoaded(null)} />
              ) : (
                <Navigate to="/" replace />
              )
            }
          />

          <Route
            path="/live"
            element={
              loaded?.kind === "live" ? (
                <Navigate to="/live/conversation" replace />
              ) : (
                <Navigate to="/" replace />
              )
            }
          />
          <Route
            path="/live/:view"
            element={
              loaded?.kind === "live" ? (
                <LiveShell connection={loaded.connection} onClose={() => setLoaded(null)} />
              ) : (
                <Navigate to="/" replace />
              )
            }
          />

          <Route path="*" element={<Navigate to="/" replace />} />
        </Routes>
      </Suspense>
    </BrowserRouter>
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

/// The landing as a route: load a file (or open a connection), then navigate into the frame that
/// serves it. Read errors stay local to this screen.
function LandingRoute({ setLoaded }: { setLoaded: (loaded: Loaded) => void }) {
  const navigate = useNavigate();
  const [error, setError] = useState<string | null>(null);
  // The file being read, so the wait on a large package shows feedback rather than a frozen page.
  const [reading, setReading] = useState<string | null>(null);

  async function openPackage(file: File) {
    setReading(file.name);
    try {
      setLoaded({ kind: "package", pkg: await loadPackageFromFile(file) });
      setError(null);
      navigate("/eval");
    } catch (cause) {
      setError(describe(file, cause));
      setReading(null);
    }
  }

  async function openHistory(file: File) {
    setReading(file.name);
    try {
      setLoaded({ kind: "history", entries: parseHistory(await file.text()) });
      setError(null);
      navigate("/trends");
    } catch (cause) {
      setError(describe(file, cause));
      setReading(null);
    }
  }

  function connectLive() {
    setLoaded({ kind: "live", connection: { baseUrl: "", key: null } });
    setError(null);
    navigate("/live");
  }

  if (reading) return <LoadingScreen label={`Reading ${reading}…`} />;

  return (
    <Landing
      onOpenPackage={openPackage}
      onOpenHistory={openHistory}
      onConnectLive={connectLive}
      error={error}
    />
  );
}

function describe(file: File, cause: unknown): string {
  const message = cause instanceof Error ? cause.message : String(cause);
  return `Could not read ${file.name} — ${message}`;
}
