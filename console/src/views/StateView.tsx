import { useState } from "react";

import type { Event } from "../types/Event.ts";
import type { Replica } from "../lib/replica.ts";
import { MemoryBrowser } from "../components/MemoryBrowser.tsx";

/// The State view: the materialized graph as it stands at the timeline cursor, browsed memory by
/// memory. The Shell folds the replica to `cursor`; keying the browser by it re-queries at that
/// fold, while selection is held here so it survives the remount. `events` carries the log-only
/// records the graph does not hold (a memory's belief arbitrations), surfaced beside the memory.
export function StateView({
  replica,
  events,
  cursor,
  onShowEvents,
}: {
  replica: Replica;
  events: Event[];
  cursor: number;
  onShowEvents?: (id: string, name: string) => void;
}) {
  const [selected, setSelected] = useState<string | null>(null);
  return (
    <MemoryBrowser
      key={cursor}
      replica={replica}
      events={events}
      cursor={cursor}
      selected={selected}
      onSelect={setSelected}
      onShowEvents={onShowEvents}
    />
  );
}
