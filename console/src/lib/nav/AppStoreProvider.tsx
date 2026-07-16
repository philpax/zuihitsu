import { useEffect, useRef, useState, type ReactNode } from "react";

import type { EvalPackage } from "@zuihitsu/wire/types/EvalPackage.ts";
import type { RunRecord } from "@zuihitsu/wire/types/RunRecord.ts";
import type { LiveConnection } from "../api/live.ts";
import { type LiveEvalConnection, fetchRunRecord, useLiveEval } from "../api/liveEval.ts";
import { type HistoryEntry, parseHistory } from "../model/history.ts";
import { summarizePackage } from "../model/packageSummary.ts";
import { loadPackageFromFile } from "../replica/package.ts";
import { type AppStore, AppStoreContext } from "./appStore.ts";

/// Holds the console's state and exposes it through context, so the static route tree can render from
/// it. `navigate` is the router's imperative navigation, threaded in from the app root — the provider
/// wraps the `RouterProvider`, so it sits outside the router's own hook context and cannot call
/// `useNavigate`. While a file is being read (a soak package is tens of megabytes, so the parse is not
/// instant), a calm loading screen stands in for the router.
export function AppStoreProvider({
  autoWatchEval = false,
  navigate,
  children,
}: {
  /// Point at the same-origin live eval on mount — the eval binary serves the console this way.
  autoWatchEval?: boolean;
  navigate: (to: string) => void;
  children: ReactNode;
}) {
  const [pkg, setPkg] = useState<EvalPackage | null>(null);
  const [pkgName, setPkgName] = useState<string | null>(null);
  const [history, setHistory] = useState<HistoryEntry[] | null>(null);
  const [live, setLive] = useState<LiveConnection | null>(null);
  const [liveEvalConn, setLiveEvalConn] = useState<LiveEvalConnection | null>(
    autoWatchEval ? { baseUrl: "" } : null,
  );
  const liveEval = useLiveEval(liveEvalConn);
  const [error, setError] = useState<string | null>(null);
  const [reading, setReading] = useState<string | null>(null);

  // Open the live eval as the initial view in eval mode — once. The landing stays reachable at "/"
  // for opening a past package to compare; only the first screen changes.
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

  const fileSummary = pkg ? summarizePackage(pkg) : null;
  const getLiveRun = (scenario: number, run: number): Promise<RunRecord> =>
    fetchRunRecord(liveEvalConn?.baseUrl ?? "", scenario, run);
  const getFileRun = (scenario: number, run: number): Promise<RunRecord> => {
    const record = pkg?.scenarios[scenario]?.runs.find((entry) => entry.index === run);
    return record
      ? Promise.resolve(record)
      : Promise.reject(new Error(`no run ${scenario}:${run} in the loaded package`));
  };

  const store: AppStore = {
    fileSummary,
    fileName: pkgName,
    history,
    live,
    liveEvalConn,
    liveEval,
    error,
    hasPackage: pkg !== null,
    hasHistory: history !== null,
    getLiveRun,
    getFileRun,
    openPackage,
    openHistory,
    connectLive,
    watchEval,
    closePackage: () => {
      setPkg(null);
      setPkgName(null);
    },
    closeHistory: () => setHistory(null),
    closeLive: () => setLive(null),
    closeLiveEval: () => setLiveEvalConn(null),
  };

  // Reading a large package (a soak file is tens of megabytes) takes a beat. Overlay the progress
  // rather than replacing `children` with it: the router is a long-lived singleton, so tearing it down
  // and remounting it — losing its state and interrupting the `navigate` fired when the read lands —
  // just to show a spinner would be a needless churn. Keep it mounted underneath the veil.
  return (
    <AppStoreContext.Provider value={store}>
      {children}
      {reading && (
        <div
          role="status"
          className="fixed inset-0 z-50 flex items-center justify-center bg-paper/90 text-sm text-ink-faint backdrop-blur-sm"
        >
          Reading {reading}…
        </div>
      )}
    </AppStoreContext.Provider>
  );
}

function describe(file: File, cause: unknown): string {
  const message = cause instanceof Error ? cause.message : String(cause);
  return `Could not read ${file.name} — ${message}`;
}
