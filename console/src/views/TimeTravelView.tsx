import { useEffect, useState } from "react";

import type { Event } from "../types/Event.ts";
import type { Replica } from "../lib/replica.ts";
import { eventSummary } from "../lib/events.ts";
import { MemoryBrowser } from "../components/MemoryBrowser.tsx";
import { Eyebrow } from "../components/primitives.tsx";

/// The Time-travel view: scrub the fold horizon and watch the graph rebuild. The bridge's foldTo
/// re-folds the replica to any earlier seq, and the memory browser below re-queries at that point —
/// remounted on each move (keyed by seq) because the replica's identity is stable, so the React
/// Compiler would otherwise cache the queries. Selection is lifted here so it survives the remount,
/// and the fold is restored to head on unmount so the other views see the latest state.
export function TimeTravelView({ replica, events }: { replica: Replica; events: Event[] }) {
  const head = replica.headSeq;
  const [seq, setSeq] = useState(head);
  const [selected, setSelected] = useState<string | null>(null);

  useEffect(() => {
    const restore = () => replica.foldTo(replica.headSeq);
    return restore;
  }, [replica]);

  function scrub(next: number) {
    replica.foldTo(next);
    setSeq(next);
  }

  const current = events.find((event) => event.seq === seq) ?? null;
  const nameById = namesUpTo(events, seq);

  return (
    <div>
      <div className="mb-10">
        <div className="mb-2 flex items-baseline justify-between">
          <Eyebrow>
            seq {seq} / {head}
          </Eyebrow>
          <div className="flex items-baseline gap-2 font-mono text-2xs">
            {current ? (
              <>
                <span className="text-ink">{current.payload.type}</span>
                <span className="text-ink-faint">{eventSummary(current.payload, nameById)}</span>
              </>
            ) : (
              <span className="text-ink-faint">before the log begins</span>
            )}
          </div>
        </div>
        <input
          type="range"
          min={0}
          max={head}
          value={seq}
          onChange={(event) => scrub(Number(event.target.value))}
          className="w-full accent-clay"
        />
      </div>

      <MemoryBrowser key={seq} replica={replica} selected={selected} onSelect={setSelected} />
    </div>
  );
}

/// The id → handle map as it stood at `seq`, built from the create and rename events up to that
/// point. Fold-independent (so it stays reactive to the scrubber) and enough to name the event line.
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
