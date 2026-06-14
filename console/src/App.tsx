import { useState } from "react";

import type { EvalPackage } from "./types/EvalPackage.ts";
import type { LiveConnection } from "./lib/live.ts";
import { type HistoryEntry, parseHistory } from "./lib/history.ts";
import { loadPackageFromFile } from "./lib/package.ts";
import { Landing } from "./components/Landing.tsx";
import { Shell } from "./components/Shell.tsx";
import { LiveShell } from "./components/LiveShell.tsx";
import { TrendsScreen } from "./components/TrendsScreen.tsx";

/// What the console has open: an eval package (one run suite, the deep views), the metrics history
/// (trends over time), or a live connection to a running agent (the same deep views, tailed).
type Loaded =
  | { kind: "package"; pkg: EvalPackage }
  | { kind: "history"; entries: HistoryEntry[] }
  | { kind: "live"; connection: LiveConnection };

/// The root: hold what is open, and route between the empty state, the package frame, and trends.
export function App() {
  const [loaded, setLoaded] = useState<Loaded | null>(null);
  const [error, setError] = useState<string | null>(null);

  async function openPackage(file: File) {
    try {
      setLoaded({ kind: "package", pkg: await loadPackageFromFile(file) });
      setError(null);
    } catch (cause) {
      setError(describe(file, cause));
    }
  }

  async function openHistory(file: File) {
    try {
      setLoaded({ kind: "history", entries: parseHistory(await file.text()) });
      setError(null);
    } catch (cause) {
      setError(describe(file, cause));
    }
  }

  function connectLive() {
    setLoaded({ kind: "live", connection: { baseUrl: "", key: null } });
    setError(null);
  }

  if (!loaded) {
    return (
      <Landing
        onOpenPackage={openPackage}
        onOpenHistory={openHistory}
        onConnectLive={connectLive}
        error={error}
      />
    );
  }
  if (loaded.kind === "package") {
    return <Shell pkg={loaded.pkg} onClose={() => setLoaded(null)} />;
  }
  if (loaded.kind === "live") {
    return <LiveShell connection={loaded.connection} onClose={() => setLoaded(null)} />;
  }
  return <TrendsScreen entries={loaded.entries} onClose={() => setLoaded(null)} />;
}

function describe(file: File, cause: unknown): string {
  const message = cause instanceof Error ? cause.message : String(cause);
  return `Could not read ${file.name} — ${message}`;
}
