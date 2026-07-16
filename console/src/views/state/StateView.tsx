import { useNavigate } from "@tanstack/react-router";

import type { EntryId } from "@zuihitsu/wire/types/EntryId.ts";
import type { Event } from "@zuihitsu/wire/types/Event.ts";
import type { Replica } from "../../lib/replica/replica.ts";
import { useSelection, useSeq, useStreamBase } from "../../lib/nav/useStreamLocation.ts";
import { eventsPath, statePath } from "../../lib/nav/routes.ts";
import { MemoryBrowser } from "./MemoryBrowser.tsx";

/// The State view: the materialized graph as it stands at the timeline cursor, browsed memory by
/// memory. The Shell folds the replica to `cursor`; keying the browser by it re-queries at that
/// fold, while the open memory rides in the URL as the `:selection` segment so it survives the
/// remount, deep-links from an event's memory ref, and moves with browser back and forward (each
/// selection is a `push`). The "events touching this memory" jump navigates to the Events view with
/// the memory pinned in `?focus`, so that jump is shareable and reversible like the rest. `events`
/// carries the log-only records the graph does not hold, surfaced beside the memory.
export function StateView({
  replica,
  events,
  cursor,
  onEditSelf,
  onRetract,
}: {
  replica: Replica;
  events: Event[];
  cursor: number;
  /// Present only in the live agent frame at the head: the operator's `self`-editing callback, threaded
  /// to the `self` memory's detail pane.
  onEditSelf?: (text: string, supersedes?: EntryId) => Promise<void>;
  /// Retract a live entry under operator authority. Present only in the live agent frame at the head.
  onRetract?: (memory: string, entry: EntryId, reason: string) => Promise<void>;
}) {
  const navigate = useNavigate();
  const base = useStreamBase();
  // The open memory rides the `:selection` segment. Absent on a bare `/…/state`, where the browser
  // defaults to `self` or the first memory.
  const selected = useSelection() ?? null;
  const seq = useSeq();

  // Selecting a memory is navigation, so it pushes a history entry (back returns to the prior memory).
  function onSelect(name: string) {
    navigate(statePath(base, name, seq));
  }

  // Jump to the Events view, filtered to the events touching this memory (carried in `focus`).
  function showEvents(id: string) {
    navigate(eventsPath(base, { focus: id, seq }));
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
      onEditSelf={onEditSelf}
      onRetract={onRetract}
    />
  );
}
