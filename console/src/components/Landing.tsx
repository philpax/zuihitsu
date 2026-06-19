import { type ReactNode, useState } from "react";

import { Eyebrow } from "./primitives.tsx";

/// The empty state: a single stacked list of ways into the console. Every entry shares one row shape,
/// so they read at equal weight and the page never reflows as you look between them — connect to a
/// running agent, watch a live eval, or open a finished package or a history.
export function Landing({
  onOpenPackage,
  onOpenHistory,
  onConnectLive,
  onWatchEval,
  error,
}: {
  onOpenPackage: (file: File) => void;
  onOpenHistory: (file: File) => void;
  onConnectLive: () => void;
  onWatchEval: (baseUrl: string) => void;
  error: string | null;
}) {
  const [url, setUrl] = useState("http://localhost:7878");

  return (
    <div className="mx-auto flex min-h-screen max-w-[40rem] flex-col justify-center px-8">
      <div className="mb-9 flex flex-col items-center gap-2">
        <span className="font-serif text-4xl text-ink">zuihitsu</span>
        <Eyebrow>console</Eyebrow>
      </div>

      <div className="border-t border-line">
        <Choice label="Connect to the agent" hint="Tail a running instance live as it thinks.">
          <button onClick={onConnectLive} className="transition-colors hover:text-clay">
            connect →
          </button>
        </Choice>

        <Choice label="Watch a live eval" hint="Follow a run as the harness drives it.">
          <form
            onSubmit={(event) => {
              event.preventDefault();
              if (url.trim()) onWatchEval(url);
            }}
            className="flex items-baseline gap-2 text-ink-faint"
          >
            <input
              value={url}
              onChange={(event) => setUrl(event.target.value)}
              spellCheck={false}
              aria-label="live eval address"
              className="w-40 border-b border-line bg-transparent py-0.5 text-right text-ink-soft outline-none transition-colors focus:border-clay"
            />
            <button
              type="submit"
              title="Watch this live eval"
              className="transition-colors hover:text-clay"
            >
              →
            </button>
          </form>
        </Choice>

        <Choice label="Open a package" hint="Inspect a finished run, scenario by scenario.">
          <FileChoose accept="application/json,.json" onFile={onOpenPackage}>
            choose a file →
          </FileChoose>
        </Choice>

        <Choice label="Open history" hint="See metrics trend across many runs over time.">
          <FileChoose accept=".jsonl,application/json" onFile={onOpenHistory}>
            choose a file →
          </FileChoose>
        </Choice>
      </div>

      {error && <p className="mt-5 text-center font-mono text-xs text-clay">{error}</p>}
    </div>
  );
}

/// One entry point: a titled, described row with its affordance to the right. The shared shape is what
/// gives the list its even weight and its still, non-reflowing height.
function Choice({ label, hint, children }: { label: string; hint: string; children: ReactNode }) {
  return (
    <div className="flex items-baseline justify-between gap-6 border-b border-line py-5">
      <div className="min-w-0">
        <div className="text-sm text-ink">{label}</div>
        <p className="mt-1 text-xs leading-relaxed text-ink-soft">{hint}</p>
      </div>
      <div className="shrink-0 font-mono text-xs text-ink-faint">{children}</div>
    </div>
  );
}

/// A file picker rendered as a quiet inline link, for the choices that open a file.
function FileChoose({
  accept,
  onFile,
  children,
}: {
  accept: string;
  onFile: (file: File) => void;
  children: ReactNode;
}) {
  return (
    <label className="cursor-pointer whitespace-nowrap transition-colors hover:text-clay">
      {children}
      <input
        type="file"
        accept={accept}
        className="hidden"
        onChange={(event) => {
          const file = event.target.files?.[0];
          if (file) onFile(file);
        }}
      />
    </label>
  );
}
