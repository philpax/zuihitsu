import { useState } from "react";

import type { EntryId } from "@zuihitsu/wire/types/EntryId.ts";
import type { Event } from "@zuihitsu/wire/types/Event.ts";
import type { Replica } from "../../lib/replica/replica.ts";
import type { MemoryView } from "../../lib/model/graph.ts";
import { nameById } from "../../lib/model/labels.ts";
import { type RecurringItem, arbitrationsFor, recurringByMemory } from "../../lib/model/audit.ts";
import { Select } from "../../components/primitives.tsx";
import { MemoryList } from "./MemoryList.tsx";
import { MemoryDetailPane } from "./MemoryDetailPane.tsx";
import { groupByNamespace } from "./memoryUtilities.ts";

/// The two-pane memory browser shared by the State and Time-travel views: a namespace-grouped list
/// on the left, the opened memory's contents, links, and `same_as` class on the right. Selection is
/// controlled by the parent so it survives the remount the Time-travel scrubber forces on each fold.
/// The console sees everything — superseded entries and all visibilities, plainly marked, plus the
/// belief arbitrations the log records but the graph does not keep.
export function MemoryBrowser({
  replica,
  events,
  cursor,
  selected,
  onSelect,
  onShowEvents,
  onEditSelf,
}: {
  replica: Replica;
  events: Event[];
  cursor: number;
  selected: string | null;
  onSelect: (name: string) => void;
  onShowEvents?: (id: string, name: string) => void;
  onEditSelf?: (text: string, supersedes?: EntryId) => Promise<void>;
}) {
  const memories = replica.memories("");
  const names = nameById(memories);
  const recurring = recurringByMemory(events, cursor);
  const [query, setQuery] = useState("");

  if (memories.length === 0) {
    return (
      <div className="py-16 text-center text-sm text-ink-faint">
        No memories at this point in the log.
      </div>
    );
  }

  const needle = query.trim().toLowerCase();
  const listed = needle
    ? memories.filter(
        (memory) =>
          memory.name.toLowerCase().includes(needle) ||
          memory.description.toLowerCase().includes(needle) ||
          memory.tags.some((tag) => tag.toLowerCase().includes(needle)),
      )
    : memories;

  // The chosen memory, or `self`, or the first — whichever exists at this fold.
  const effective =
    (selected && memories.find((memory) => memory.name === selected)?.name) ??
    memories.find((memory) => memory.name === "self")?.name ??
    memories[0].name;
  const detail = replica.memory(effective);

  return (
    <div className="grid grid-cols-1 gap-5 md:grid-cols-[15rem_1fr] md:gap-8">
      <div className="flex flex-col gap-4 self-start">
        <input
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          placeholder="filter memories…"
          className="border-b border-line bg-transparent pb-1 font-mono text-xs text-ink placeholder:text-ink-faint/60 focus:border-ink-faint focus:outline-none"
        />
        {listed.length === 0 ? (
          <p className="font-mono text-2xs text-ink-faint">no matches</p>
        ) : (
          <>
            <MemorySelect
              memories={listed}
              selected={effective}
              recurring={recurring}
              onSelect={onSelect}
            />
            <div className="hidden md:block">
              <MemoryList
                memories={listed}
                selected={effective}
                recurring={recurring}
                onSelect={onSelect}
              />
            </div>
          </>
        )}
      </div>
      {detail ? (
        <MemoryDetailPane
          detail={detail}
          nameById={names}
          arbitrations={arbitrationsFor(events, detail.memory.id, cursor)}
          recurring={recurring.get(detail.memory.id) ?? []}
          onShowEvents={onShowEvents}
          onSelect={onSelect}
          onEditSelf={onEditSelf}
        />
      ) : (
        <div className="py-16 text-center text-sm text-ink-faint">Select a memory.</div>
      )}
    </div>
  );
}

/// The mobile face of the memory list: a native dropdown grouped by namespace, so the opened memory
/// owns the screen instead of scrolling past the whole list. Hidden once there is room for the
/// sidebar (`md`).
function MemorySelect({
  memories,
  selected,
  recurring,
  onSelect,
}: {
  memories: MemoryView[];
  selected: string | null;
  recurring: Map<string, RecurringItem[]>;
  onSelect: (name: string) => void;
}) {
  const groups = groupByNamespace(memories);
  return (
    <Select
      value={selected ?? ""}
      onChange={(event) => onSelect(event.target.value)}
      className="md:hidden"
      aria-label="Choose a memory"
    >
      {groups.map(([namespace, items]) => (
        <optgroup key={namespace} label={namespace}>
          {items.map((memory) => (
            <option key={memory.id} value={memory.name}>
              {memory.name}
              {recurring.has(memory.id) ? " ↻" : ""}
            </option>
          ))}
        </optgroup>
      ))}
    </Select>
  );
}
