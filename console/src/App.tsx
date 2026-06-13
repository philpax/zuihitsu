import { useState } from "react";

import type { EvalPackage } from "./types/EvalPackage.ts";
import { loadPackageFromFile } from "./lib/package.ts";
import { formatDate } from "./lib/format.ts";
import { ScenarioOverview } from "./views/ScenarioOverview.tsx";

/// The console's views, in the order the build plan establishes them. Only those with `ready` are
/// wired; the rest stand in the nav as the shape of what is coming.
const VIEWS = [
  { id: "scenarios", label: "Scenarios", ready: true },
  { id: "conversation", label: "Conversation", ready: false },
  { id: "events", label: "Events", ready: false },
  { id: "state", label: "State", ready: false },
  { id: "time", label: "Time-travel", ready: false },
] as const;

type ViewId = (typeof VIEWS)[number]["id"];

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

function Shell({ pkg, onClose }: { pkg: EvalPackage; onClose: () => void }) {
  const [view, setView] = useState<ViewId>("scenarios");
  const { meta } = pkg;

  return (
    <div className="mx-auto flex min-h-screen max-w-[72rem] flex-col px-8">
      <header className="flex items-baseline justify-between border-b border-line py-6">
        <div className="flex items-baseline gap-3">
          <span className="font-serif text-xl text-ink">zuihitsu</span>
          <span className="font-mono text-2xs uppercase tracking-widest text-ink-faint">
            console
          </span>
        </div>
        <div className="flex items-baseline gap-3 font-mono text-xs text-ink-soft">
          <Meta>{meta.model_id}</Meta>
          {meta.git_sha && <Meta>{meta.git_sha.slice(0, 7)}</Meta>}
          <Meta>{meta.runs_per_scenario}×/scenario</Meta>
          <Meta>{formatDate(meta.finished_at_ms)}</Meta>
          <button
            onClick={onClose}
            className="text-ink-faint transition-colors hover:text-clay"
            title="Close this package"
          >
            ✕
          </button>
        </div>
      </header>

      <nav className="flex gap-7 border-b border-line text-sm">
        {VIEWS.map((entry) => {
          const active = entry.id === view;
          return (
            <button
              key={entry.id}
              disabled={!entry.ready}
              onClick={() => entry.ready && setView(entry.id)}
              title={entry.ready ? undefined : "Coming soon"}
              className={
                "-mb-px border-b-2 py-3 transition-colors " +
                (active
                  ? "border-clay text-ink"
                  : entry.ready
                    ? "border-transparent text-ink-soft hover:text-ink"
                    : "cursor-default border-transparent text-ink-faint/60")
              }
            >
              {entry.label}
            </button>
          );
        })}
      </nav>

      <main className="flex-1 py-10">
        {view === "scenarios" && <ScenarioOverview pkg={pkg} />}
      </main>
    </div>
  );
}

function Meta({ children }: { children: React.ReactNode }) {
  return (
    <span className="flex items-baseline gap-3 before:text-ink-faint/50 before:content-['·'] first:before:hidden">
      {children}
    </span>
  );
}

function Landing({
  onOpen,
  error,
}: {
  onOpen: (file: File) => void;
  error: string | null;
}) {
  const [hovering, setHovering] = useState(false);

  return (
    <div className="mx-auto flex min-h-screen max-w-[40rem] flex-col justify-center px-8">
      <p className="mb-3 font-mono text-2xs uppercase tracking-widest text-ink-faint">
        zuihitsu · console
      </p>
      <h1 className="font-serif text-3xl text-ink">What was the agent thinking?</h1>
      <p className="mt-4 max-w-prose text-base text-ink-soft">
        Open an eval package to inspect a run end to end — its memories and their confidences, the
        rooms it spoke in, and the deliberation behind every turn. The package is a replay of the
        agent's own event log; everything here is a reconstruction from it.
      </p>

      <label
        onDragOver={(event) => {
          event.preventDefault();
          setHovering(true);
        }}
        onDragLeave={() => setHovering(false)}
        onDrop={(event) => {
          event.preventDefault();
          setHovering(false);
          const file = event.dataTransfer.files[0];
          if (file) onOpen(file);
        }}
        className={
          "mt-10 flex cursor-pointer flex-col items-center justify-center gap-2 border border-dashed py-14 transition-colors " +
          (hovering
            ? "border-clay bg-clay-soft/15 text-ink"
            : "border-line-strong text-ink-soft hover:border-ink-faint")
        }
      >
        <span className="text-base">Drop an eval package here</span>
        <span className="font-mono text-xs text-ink-faint">or choose a file</span>
        <input
          type="file"
          accept="application/json,.json"
          className="hidden"
          onChange={(event) => {
            const file = event.target.files?.[0];
            if (file) onOpen(file);
          }}
        />
      </label>

      {error && <p className="mt-5 font-mono text-xs text-clay">{error}</p>}
    </div>
  );
}
