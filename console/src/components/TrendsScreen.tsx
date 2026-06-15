import type { HistoryEntry } from "../lib/history.ts";
import { TrendsView } from "../views/TrendsView.tsx";
import { FrameNav } from "./FrameNav.tsx";

/// The frame for the metrics history — a minimal header (the history is not a package, so it has no
/// run meta or view nav) around the Trends view, with the cross-section nav to pivot to a package.
export function TrendsScreen({
  entries,
  onClose,
}: {
  entries: HistoryEntry[];
  onClose: () => void;
}) {
  return (
    <div className="mx-auto flex min-h-screen max-w-[76rem] flex-col px-4 sm:px-8">
      <header className="flex items-baseline justify-between border-b border-line py-6">
        <div className="flex items-baseline gap-3">
          <span className="font-serif text-xl text-ink">zuihitsu</span>
          <FrameNav current="trends" />
        </div>
        <button
          onClick={onClose}
          className="font-mono text-xs text-ink-faint transition-colors hover:text-clay"
          title="Close"
        >
          ✕
        </button>
      </header>

      <main className="flex-1 py-7">
        <TrendsView entries={entries} />
      </main>
    </div>
  );
}
