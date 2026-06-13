import { useState } from "react";

import { Eyebrow } from "./primitives.tsx";

/// The empty state: a calm invitation to open an eval package, by drop or by file picker.
export function Landing({ onOpen, error }: { onOpen: (file: File) => void; error: string | null }) {
  const [hovering, setHovering] = useState(false);

  return (
    <div className="mx-auto flex min-h-screen max-w-[40rem] flex-col justify-center px-8">
      <Eyebrow className="mb-3">zuihitsu · console</Eyebrow>
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
