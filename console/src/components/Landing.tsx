import { useState } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";

import { Eyebrow } from "./primitives.tsx";

type Source = "agent" | "eval";

/// The empty state: choose a source to debug. The two behave near-identically once open — the same
/// state, conversation, and event views over one stream — differing only in where the stream comes
/// from. **Agent** tails a running instance live; **Eval** loads a package of finished runs from a
/// file and lets you open any one of them.
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
  const [source, setSource] = useState<Source>("agent");
  const reduce = useReducedMotion();
  const shift = reduce ? 0 : 24;
  // Agent sits left of Eval, so Eval slides in from the right and Agent from the left.
  const direction = source === "eval" ? 1 : -1;

  return (
    <div className="mx-auto flex min-h-screen max-w-[40rem] flex-col justify-center px-8">
      <Eyebrow className="mb-3">zuihitsu · console</Eyebrow>
      <h1 className="font-serif text-3xl text-ink">What was the agent thinking?</h1>
      <p className="mt-4 max-w-prose text-base text-ink-soft">
        Inspect an agent's memories and their confidences, the rooms it spoke in, and the
        deliberation behind every turn. Everything here is a reconstruction from the agent's own
        event log.
      </p>

      <div className="mt-8 flex gap-7 border-b border-line text-sm">
        {(["agent", "eval"] as const).map((tab) => (
          <button
            key={tab}
            onClick={() => setSource(tab)}
            className={
              "-mb-px border-b-2 py-3 capitalize transition-colors " +
              (tab === source
                ? "border-clay text-ink"
                : "border-transparent text-ink-soft hover:text-ink")
            }
          >
            {tab}
          </button>
        ))}
      </div>

      <div className="relative overflow-x-clip">
        <AnimatePresence mode="popLayout" custom={direction} initial={false}>
          <motion.div
            key={source}
            initial={{ x: direction * shift, opacity: 0 }}
            animate={{ x: 0, opacity: 1 }}
            exit={{ x: direction * -shift, opacity: 0 }}
            transition={{ duration: reduce ? 0.12 : 0.28, ease: [0.32, 0.72, 0, 1] }}
          >
            {source === "agent" ? (
              <AgentPanel onConnect={onConnectLive} />
            ) : (
              <EvalPanel
                onOpenPackage={onOpenPackage}
                onOpenHistory={onOpenHistory}
                onWatchEval={onWatchEval}
              />
            )}
          </motion.div>
        </AnimatePresence>
      </div>

      {error && <p className="mt-5 text-center font-mono text-xs text-clay">{error}</p>}
    </div>
  );
}

function AgentPanel({ onConnect }: { onConnect: () => void }) {
  return (
    <div className="mt-6 flex flex-col items-center gap-4 py-7">
      <p className="max-w-prose text-center text-sm text-ink-soft">
        Tail the running instance live — its log streams in as it thinks, and the timeline grows
        with it. Scrub back to inspect any earlier moment without stopping the stream.
      </p>
      <button
        onClick={onConnect}
        className="border border-line-strong px-6 py-2.5 text-base text-ink transition-colors hover:border-clay hover:text-clay"
      >
        Connect to the agent
      </button>
    </div>
  );
}

function EvalPanel({
  onOpenPackage,
  onOpenHistory,
  onWatchEval,
}: {
  onOpenPackage: (file: File) => void;
  onOpenHistory: (file: File) => void;
  onWatchEval: (baseUrl: string) => void;
}) {
  const [hovering, setHovering] = useState(false);
  const [url, setUrl] = useState("http://localhost:7878");
  return (
    <div className="mt-6">
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
          if (file) onOpenPackage(file);
        }}
        className={
          "flex cursor-pointer flex-col items-center justify-center gap-2 border border-dashed py-10 transition-colors " +
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
            if (file) onOpenPackage(file);
          }}
        />
      </label>

      <label className="mt-6 block cursor-pointer text-center font-mono text-2xs text-ink-faint transition-colors hover:text-clay">
        or open a history file to see trends over time
        <input
          type="file"
          accept=".jsonl,application/json"
          className="hidden"
          onChange={(event) => {
            const file = event.target.files?.[0];
            if (file) onOpenHistory(file);
          }}
        />
      </label>

      <form
        onSubmit={(event) => {
          event.preventDefault();
          if (url.trim()) onWatchEval(url);
        }}
        className="mt-4 flex items-baseline justify-center gap-2 font-mono text-2xs text-ink-faint"
      >
        <span>or watch a live eval at</span>
        <input
          value={url}
          onChange={(event) => setUrl(event.target.value)}
          spellCheck={false}
          aria-label="live eval address"
          className="w-44 border-b border-line bg-transparent py-0.5 text-ink-soft outline-none transition-colors focus:border-clay"
        />
        <button
          type="submit"
          title="Watch this live eval"
          className="transition-colors hover:text-clay"
        >
          →
        </button>
      </form>
    </div>
  );
}
