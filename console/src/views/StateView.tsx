import { useSearchParams } from "react-router-dom";

import type { Event } from "../types/Event.ts";
import type { Replica } from "../lib/replica.ts";
import { MemoryBrowser } from "../components/MemoryBrowser.tsx";

/// The State view: the materialized graph as it stands at the timeline cursor, browsed memory by
/// memory. The Shell folds the replica to `cursor`; keying the browser by it re-queries at that
/// fold, while the open memory rides in the URL (`?memory`) so it survives the remount, and so an
/// event's memory ref can deep-link straight to it. `events` carries the log-only records the graph
/// does not hold (a memory's belief arbitrations), surfaced beside the memory.
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
  const [searchParams, setSearchParams] = useSearchParams();
  const selected = searchParams.get("memory");

  function onSelect(name: string) {
    setSearchParams(
      (prev) => {
        const updated = new URLSearchParams(prev);
        updated.set("memory", name);
        return updated;
      },
      { replace: true },
    );
  }

  return (
    <MemoryBrowser
      key={cursor}
      replica={replica}
      events={events}
      cursor={cursor}
      selected={selected}
      onSelect={onSelect}
      onShowEvents={onShowEvents}
    />
  );
}
