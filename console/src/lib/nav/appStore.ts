import { createContext, useContext } from "react";

import type { RunRecord } from "@zuihitsu/wire/types/RunRecord.ts";
import type { PackageSummary } from "@zuihitsu/wire/types/PackageSummary.ts";
import type { LiveConnection } from "../api/live.ts";
import type { LiveEvalConnection, useLiveEval } from "../api/liveEval.ts";
import type { HistoryEntry } from "../model/history.ts";

/// The console's mutable state, lifted out of the screens. The screens are chosen by a `switch` over the current location, so a
/// screen cannot close over React state directly; instead the whole console's state and its actions live in the
/// [`AppStoreProvider`], and each screen reads what it needs through [`useAppStore`]. The
/// package, the metrics history, and a live connection are held *independently* — they are the same
/// eval seen different ways, not mutually exclusive modes — so any frame can pivot to a sibling
/// section without returning to the landing.
export interface AppStore {
  /// The file-loaded package's lean summary (verdicts and metrics render from it; the full log is
  /// fetched per run through `getFileRun`). `null` when no file package is open.
  fileSummary: PackageSummary | null;
  /// The file package's name, for the header — the package itself does not carry it.
  fileName: string | null;
  history: HistoryEntry[] | null;
  live: LiveConnection | null;
  liveEvalConn: LiveEvalConnection | null;
  liveEval: ReturnType<typeof useLiveEval>;
  error: string | null;

  hasPackage: boolean;
  hasHistory: boolean;

  /// The deep-dive's fetch seam: a live eval reads a run's full record over the harness's run
  /// endpoint; a file-loaded one resolves it from the retained full package.
  getLiveRun: (scenario: number, run: number) => Promise<RunRecord>;
  getFileRun: (scenario: number, run: number) => Promise<RunRecord>;

  openPackage: (file: File) => Promise<void>;
  openHistory: (file: File) => Promise<void>;
  connectLive: (baseUrl: string) => void;
  watchEval: (baseUrl: string) => void;
  closePackage: () => void;
  closeHistory: () => void;
  closeLive: () => void;
  closeLiveEval: () => void;
}

export const AppStoreContext = createContext<AppStore | null>(null);

export function useAppStore(): AppStore {
  const store = useContext(AppStoreContext);
  if (!store) throw new Error("useAppStore must be used within the AppStoreProvider");
  return store;
}
