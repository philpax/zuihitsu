import type { Event } from "../types/Event.ts";
import { eventSummary } from "../lib/events.ts";
import { Eyebrow } from "./primitives.tsx";

/// The global time cursor for the run-scoped views: a sticky scrubber over the run's seq range that
/// every view reflects — the State graph folds to it, the Conversation and Events stop there. The
/// event at the cursor narrates the move; the readout doubles as a reset to the head.
export function Timeline({
  head,
  seq,
  events,
  onScrub,
  onReset,
}: {
  head: number;
  seq: number;
  events: Event[];
  onScrub: (seq: number) => void;
  onReset: () => void;
}) {
  const current = events.find((event) => event.seq === seq) ?? null;
  const nameById = namesUpTo(events, seq);
  const atHead = seq >= head;

  return (
    <footer className="sticky bottom-0 border-t border-line bg-paper/95 py-3 backdrop-blur">
      <div className="mb-1.5 flex items-baseline justify-between gap-4">
        <Eyebrow>timeline</Eyebrow>
        <div className="flex min-w-0 items-baseline gap-2 font-mono text-2xs">
          <button
            onClick={onReset}
            className={"shrink-0 " + (atHead ? "text-ink-faint" : "text-clay hover:text-ink")}
            title={atHead ? "At the latest state" : "Jump to the latest state"}
          >
            seq {seq} / {head}
          </button>
          {current && (
            <span className="truncate text-ink-faint">
              · {current.payload.type} {eventSummary(current.payload, nameById)}
            </span>
          )}
        </div>
      </div>
      <input
        type="range"
        min={0}
        max={head}
        value={seq}
        onChange={(event) => onScrub(Number(event.target.value))}
        className="w-full accent-clay"
      />
    </footer>
  );
}

/// The id → handle map as it stood at `seq`, built from the create and rename events up to that
/// point — enough to name the event at the cursor without depending on the current fold.
function namesUpTo(events: Event[], seq: number): Map<string, string> {
  const names = new Map<string, string>();
  for (const event of events) {
    if (event.seq > seq) break;
    if (event.payload.type === "MemoryCreated") names.set(event.payload.id, event.payload.name);
    else if (event.payload.type === "MemoryRenamed")
      names.set(event.payload.id, event.payload.new_name);
  }
  return names;
}
