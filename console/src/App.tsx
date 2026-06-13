import { useState } from "react";

import type { EvalPackage } from "./types/EvalPackage.ts";
import { type HistoryEntry, parseHistory } from "./lib/history.ts";
import { loadPackageFromFile } from "./lib/package.ts";
import { Landing } from "./components/Landing.tsx";
import { Shell } from "./components/Shell.tsx";
import { TrendsScreen } from "./components/TrendsScreen.tsx";

/// Either an eval package (one run suite, the deep views) or the metrics history (trends over time).
type Loaded = { kind: "package"; pkg: EvalPackage } | { kind: "history"; entries: HistoryEntry[] };

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

  if (!loaded) {
    return <Landing onOpenPackage={openPackage} onOpenHistory={openHistory} error={error} />;
  }
  if (loaded.kind === "package") {
    return <Shell pkg={loaded.pkg} onClose={() => setLoaded(null)} />;
  }
  return <TrendsScreen entries={loaded.entries} onClose={() => setLoaded(null)} />;
}

function describe(file: File, cause: unknown): string {
  const message = cause instanceof Error ? cause.message : String(cause);
  return `Could not read ${file.name} — ${message}`;
}
