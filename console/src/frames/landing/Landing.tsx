import { type ReactNode, useState } from "react";

import { useDocumentTitle } from "../../lib/nav/useDocumentTitle.ts";
import { Eyebrow } from "../../components/primitives.tsx";

/// The empty state: a single stacked list of ways into the console. Every entry shares one row shape,
/// so they read at equal weight and the page never reflows as you look between them — connect to a
/// running agent, watch live runs, open saved runs, or see how the recordings trend.
export function Landing({
  onOpenPackage,
  onOpenHistory,
  onConnectLive,
  onWatchEval,
  error,
}: {
  onOpenPackage: (file: File) => void;
  onOpenHistory: (file: File) => void;
  onConnectLive: (baseUrl: string) => void;
  onWatchEval: (baseUrl: string) => void;
  error: string | null;
}) {
  useDocumentTitle("console");
  return (
    <div className="mx-auto flex min-h-screen max-w-160 flex-col justify-center px-5 py-10 sm:px-8">
      <div className="mb-9 flex flex-col items-center gap-2">
        <span className="font-serif text-4xl text-ink">zuihitsu</span>
        <Eyebrow>console</Eyebrow>
      </div>

      <div className="border-t border-line">
        <Choice label="Connect to an agent" hint="Interact with and manage a running agent.">
          <UrlSubmit
            initial={window.location.origin}
            aria="agent address"
            onSubmit={onConnectLive}
          />
        </Choice>

        <Choice label="Live runs" hint="Watch an eval suite as the harness runs it.">
          <UrlSubmit
            initial="http://localhost:7878"
            aria="live eval address"
            onSubmit={onWatchEval}
          />
        </Choice>

        <Choice label="Saved runs" hint="Open a finished eval suite from a file.">
          <FileChoose accept="application/json,.json" onFile={onOpenPackage}>
            choose a file →
          </FileChoose>
        </Choice>

        <Choice label="Trends" hint="See how the recordings trend across runs over time.">
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
        <p className="mt-1 text-xs/relaxed text-ink-soft">{hint}</p>
      </div>
      <div className="shrink-0 font-mono text-xs text-ink-faint">{children}</div>
    </div>
  );
}

/// An address field and a submit arrow, for the choices that connect to a stream by URL.
function UrlSubmit({
  initial,
  aria,
  onSubmit,
}: {
  initial: string;
  aria: string;
  onSubmit: (baseUrl: string) => void;
}) {
  const [url, setUrl] = useState(initial);
  return (
    <form
      onSubmit={(event) => {
        event.preventDefault();
        if (url.trim()) onSubmit(url);
      }}
      className="flex items-baseline gap-2 text-ink-faint"
    >
      <input
        value={url}
        onChange={(event) => setUrl(event.target.value)}
        spellCheck={false}
        aria-label={aria}
        className="w-44 border-b border-line bg-transparent py-0.5 text-right text-ink-soft transition-colors outline-none focus:border-clay"
      />
      <button type="submit" title="Connect" className="transition-colors hover:text-clay">
        →
      </button>
    </form>
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
