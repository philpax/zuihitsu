import { useNavigate, useSearchParams } from "react-router-dom";

import type { Event } from "../../types/Event.ts";
import type { Replica } from "../../lib/replica/replica.ts";
import { useStreamBase } from "../../lib/nav/useStreamLocation.ts";
import { MemoryBrowser } from "./MemoryBrowser.tsx";

/// The State view: the materialized graph as it stands at the timeline cursor, browsed memory by
/// memory. The Shell folds the replica to `cursor`; keying the browser by it re-queries at that
/// fold, while the open memory rides in the URL (`?memory`) so it survives the remount, and so an
/// event's memory ref can deep-link straight to it. The "events touching this memory" jump navigates
/// to the Events view with the memory pinned in `?focus`, so that jump is shareable and reversible
/// like the rest. `events` carries the log-only records the graph does not hold, surfaced beside the
/// memory.
export function StateView({
  replica,
  events,
  cursor,
}: {
  replica: Replica;
  events: Event[];
  cursor: number;
}) {
  const navigate = useNavigate();
  const base = useStreamBase();
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

  // Jump to the Events view, filtered to the events touching this memory (carried in ?focus). The
  // cursor is preserved so the events stop at the same point in the timeline.
  function showEvents(id: string) {
    const params = new URLSearchParams();
    const seq = searchParams.get("seq");
    if (seq) params.set("seq", seq);
    params.set("focus", id);
    navigate(`${base}/events?${params.toString()}`);
  }

  return (
    <MemoryBrowser
      key={cursor}
      replica={replica}
      events={events}
      cursor={cursor}
      selected={selected}
      onSelect={onSelect}
      onShowEvents={showEvents}
    />
  );
}
