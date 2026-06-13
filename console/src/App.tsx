import { useState } from "react";

import type { EvalPackage } from "./types/EvalPackage.ts";
import { loadPackageFromFile } from "./lib/package.ts";
import { Landing } from "./components/Landing.tsx";
import { Shell } from "./components/Shell.tsx";

/// The root: hold the loaded package, and route between the empty state and the loaded frame.
export function App() {
  const [pkg, setPkg] = useState<EvalPackage | null>(null);
  const [error, setError] = useState<string | null>(null);

  async function open(file: File) {
    try {
      setPkg(await loadPackageFromFile(file));
      setError(null);
    } catch (cause) {
      const message = cause instanceof Error ? cause.message : String(cause);
      setError(`Could not read ${file.name} — ${message}`);
    }
  }

  if (!pkg) return <Landing onOpen={open} error={error} />;
  return <Shell pkg={pkg} onClose={() => setPkg(null)} />;
}
