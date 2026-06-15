import { createContext, useContext } from "react";

/// What the frame chrome needs to move between the package's deep views and the metrics trends without
/// returning to the landing: whether each is loaded, and a loader to open one on demand. The two are
/// the same eval seen two ways — in depth and over time — so the console holds them side by side
/// rather than as mutually exclusive modes.
export interface ConsoleNav {
  hasPackage: boolean;
  hasHistory: boolean;
  openPackage: (file: File) => void;
  openHistory: (file: File) => void;
}

export const ConsoleNavContext = createContext<ConsoleNav | null>(null);

export function useConsoleNav(): ConsoleNav {
  const nav = useContext(ConsoleNavContext);
  if (!nav) throw new Error("useConsoleNav must be used within the console");
  return nav;
}
