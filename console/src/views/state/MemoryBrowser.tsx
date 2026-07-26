import { useState } from "react";

import type { EntryId } from "@zuihitsu/wire/types/EntryId.ts";
import type { Event } from "@zuihitsu/wire/types/Event.ts";
import type { MemoryId } from "@zuihitsu/wire/types/MemoryId.ts";
import type { Replica } from "../../lib/replica/replica.ts";
import type { MemoryView } from "@zuihitsu/wire/types/MemoryView.ts";
import { nameById } from "../../lib/model/labels.ts";
import { type RecurringItem, arbitrationsFor } from "../../lib/model/audit.ts";
import { Select } from "../../components/primitives.tsx";
import { MemoryList } from "./MemoryList.tsx";
import { MemoryDetailPane } from "./MemoryDetailPane.tsx";
import { clusterByClass, groupByNamespace } from "./memoryUtilities.ts";

/// The two-pane memory browser behind the State view: a namespace-grouped list on the left, the
/// opened memory's contents, links, and `same_as` class on the right. Selection is controlled by
/// the parent so it survives the remount the timeline scrubber forces on each fold.
/// The console sees everything — superseded entries and all visibilities, plainly marked, plus the
/// belief arbitrations the log records but the graph does not keep.
export function MemoryBrowser({
  replica,
  events,
  cursor,
  selected,
  onSelect,
  onShowEvents,
  readOnly = false,
  onEditSelf,
  onRetract,
}: {
  replica: Replica;
  events: Event[];
  cursor: number;
  selected: string | null;
  onSelect: (name: string) => void;
  onShowEvents?: (id: string, name: string) => void;
  /// Whether the instance is booted for inspection only: the operator affordances below render but
  /// hold their actions closed.
  readOnly?: boolean;
  onEditSelf?: (text: string, supersedes?: EntryId) => Promise<void>;
  onRetract?: (memory: string, entry: EntryId, reason: string) => Promise<void>;
}) {
  const memories = replica.memories("");
  const names = nameById(memories);
  // Each memory's canonical `same_as` primary, from the graph's projection — the key the sidebar
  // clusters an identity class on, plus the operator-pinned designations it marks.
  const classes = replica.memoryClasses();
  const primaryOf = new Map<MemoryId, MemoryId>(classes.map((cls) => [cls.id, cls.primary]));
  const designated = new Set<MemoryId>(
    classes.filter((cls) => cls.designated).map((cls) => cls.id),
  );
  // Which memories carry a live recurring occurrence, from the graph's projection (not a re-fold of
  // the log): the replica is the authority, grouped here into the per-memory shape the list badges.
  const recurring = new Map<string, RecurringItem[]>();
  for (const entry of replica.recurringEntries()) {
    const items = recurring.get(entry.memory) ?? [];
    items.push({ text: entry.text, rrule: entry.rrule });
    recurring.set(entry.memory, items);
  }
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

  // On `md` the grid fills the well's definite height and each pane scrolls itself — the sidebar
  // never grows the main body, and no script measures anything; below `md` the panes stack and the
  // well scrolls as one.
  return (
    <div className="grid grid-cols-1 gap-5 md:h-full md:grid-cols-[15rem_1fr] md:grid-rows-[minmax(0,1fr)] md:gap-8">
      <div className="flex flex-col gap-4 self-start md:min-h-0 md:self-stretch">
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
              primaryOf={primaryOf}
              onSelect={onSelect}
            />
            <div className="hidden md:block md:min-h-0 md:overflow-y-auto md:pr-1">
              <MemoryList
                memories={listed}
                selected={effective}
                recurring={recurring}
                primaryOf={primaryOf}
                designated={designated}
                onSelect={onSelect}
              />
            </div>
          </>
        )}
      </div>
      <div className="md:min-h-0 md:overflow-y-auto">
        {detail ? (
          <MemoryDetailPane
            detail={detail}
            nameById={names}
            arbitrations={arbitrationsFor(events, detail.memory.id, cursor)}
            recurring={recurring.get(detail.memory.id) ?? []}
            onShowEvents={onShowEvents}
            readOnly={readOnly}
            onEditSelf={onEditSelf}
            onRetract={onRetract}
          />
        ) : (
          <div className="py-16 text-center text-sm text-ink-faint">Select a memory.</div>
        )}
      </div>
    </div>
  );
}

/// The mobile face of the memory list: a native dropdown grouped by namespace, so the opened memory
/// owns the screen instead of scrolling past the whole list. Hidden once there is room for the
/// sidebar (`md`). A `same_as` class's members follow their canonical primary, indented — a native
/// `<option>` cannot nest, so the clustering reads through order and a leading mark rather than a
/// collapsible node.
function MemorySelect({
  memories,
  selected,
  recurring,
  primaryOf,
  onSelect,
}: {
  memories: MemoryView[];
  selected: string | null;
  recurring: Map<string, RecurringItem[]>;
  primaryOf: Map<MemoryId, MemoryId>;
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
          {clusterByClass(items, primaryOf).flatMap(({ primary, members }) => [
            <option key={primary.id} value={primary.name}>
              {primary.name}
              {recurring.has(primary.id) ? " ↻" : ""}
            </option>,
            ...members.map((member) => (
              <option key={member.id} value={member.name}>
                {`↳ ${member.name}`}
                {recurring.has(member.id) ? " ↻" : ""}
              </option>
            )),
          ])}
        </optgroup>
      ))}
    </Select>
  );
}
