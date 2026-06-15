import { useState } from "react";

import type { Event } from "../types/Event.ts";
import type { Replica } from "../lib/replica.ts";
import type { EntryView, MemoryDetail, MemoryView } from "../lib/graph.ts";
import { isPrivate, nameById, tellerLabel, visibilityLabel } from "../lib/labels.ts";
import { formatDateTime } from "../lib/format.ts";
import {
  type Arbitration,
  type RecurringItem,
  arbitrationsFor,
  recurringByMemory,
  rruleLabel,
} from "../lib/audit.ts";
import { groupBy } from "../lib/collections.ts";
import { Eyebrow } from "./primitives.tsx";

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
}: {
  replica: Replica;
  events: Event[];
  cursor: number;
  selected: string | null;
  onSelect: (name: string) => void;
  onShowEvents?: (id: string, name: string) => void;
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
    <select
      value={selected ?? ""}
      onChange={(event) => onSelect(event.target.value)}
      className="w-full border border-line bg-paper px-3 py-2 font-mono text-xs text-ink focus:border-ink-faint focus:outline-none md:hidden"
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
    </select>
  );
}

function MemoryList({
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
    <nav className="flex flex-col gap-4 sm:gap-6">
      {groups.map(([namespace, items]) => (
        <div key={namespace}>
          <Eyebrow>{namespace}</Eyebrow>
          <ul className="mt-2 flex flex-col">
            {items.map((memory) => {
              const active = memory.name === selected;
              return (
                <li key={memory.id}>
                  <button
                    onClick={() => onSelect(memory.name)}
                    title={memory.name}
                    className={
                      "-ml-3 flex w-full min-w-0 items-baseline border-l-2 py-1 pl-2.5 text-left font-mono text-xs transition-colors " +
                      (active
                        ? "border-clay text-ink"
                        : "border-transparent text-ink-soft hover:text-ink")
                    }
                  >
                    <span className="truncate">{leafName(memory.name, namespace)}</span>
                    {recurring.has(memory.id) && (
                      <span className="ml-1.5 shrink-0 text-sage" title="recurring">
                        ↻
                      </span>
                    )}
                  </button>
                </li>
              );
            })}
          </ul>
        </div>
      ))}
    </nav>
  );
}

function MemoryDetailPane({
  detail,
  nameById,
  arbitrations,
  recurring,
  onShowEvents,
}: {
  detail: MemoryDetail;
  nameById: Map<string, string>;
  arbitrations: Arbitration[];
  recurring: RecurringItem[];
  onShowEvents?: (id: string, name: string) => void;
}) {
  const { memory, entries, history, links } = detail;
  const superseded = history.filter((entry) => entry.superseded_by !== null);
  const classPeers = detail.class.filter((peer) => peer.id !== memory.id);

  return (
    <article className="max-w-prose">
      <header className="border-b border-line pb-5">
        <div className="flex items-baseline justify-between gap-4">
          {/* On mobile the dropdown already names the open memory, so the heading would just repeat it. */}
          <h2 className="hidden font-mono text-base text-ink md:block">{memory.name}</h2>
          <div className="flex items-baseline gap-4">
            {onShowEvents && (
              <button
                onClick={() => onShowEvents(memory.id, memory.name)}
                className="shrink-0 font-mono text-2xs text-clay transition-colors hover:text-ink"
                title="Show every event touching this memory"
              >
                events ↗
              </button>
            )}
            <Eyebrow>{memory.volatility} volatility</Eyebrow>
          </div>
        </div>
        {memory.tags.length > 0 && (
          <div className="mt-3 flex flex-wrap gap-1.5">
            {memory.tags.map((tag) => (
              <span
                key={tag}
                className="border border-sage-soft px-1.5 py-0.5 font-mono text-2xs text-sage"
              >
                #{tag}
              </span>
            ))}
          </div>
        )}
        {memory.description && (
          <p className="mt-4 font-serif text-base leading-relaxed text-ink-soft">
            {memory.description}
          </p>
        )}
        {classPeers.length > 0 && (
          <p className="mt-3 font-mono text-2xs text-ink-faint">
            same as {classPeers.map((peer) => peer.name).join(", ")}
          </p>
        )}
        <p className="mt-3 font-mono text-2xs text-ink-faint">
          created {formatDateTime(memory.created_at)}
        </p>
      </header>

      <Section label={`contents · ${entries.length}`}>
        {entries.length === 0 ? (
          <p className="text-sm text-ink-faint">No live entries.</p>
        ) : (
          <ul className="flex flex-col gap-4">
            {entries.map((entry) => (
              <EntryItem key={entry.entry_id} entry={entry} nameById={nameById} />
            ))}
          </ul>
        )}
      </Section>

      {links.length > 0 && (
        <Section label={`links · ${links.length}`}>
          <ul className="flex flex-col gap-1.5 font-mono text-xs text-ink-soft">
            {links.map((link, index) => (
              <li key={index} className="flex items-baseline gap-2">
                <span className="text-clay">{link.relation}</span>
                <span className="text-ink-faint">→</span>
                <span>{nameById.get(link.to) ?? link.to}</span>
              </li>
            ))}
          </ul>
        </Section>
      )}

      {superseded.length > 0 && (
        <Section label={`superseded · ${superseded.length}`}>
          <ul className="flex flex-col gap-4">
            {superseded.map((entry) => (
              <EntryItem key={entry.entry_id} entry={entry} nameById={nameById} faded />
            ))}
          </ul>
        </Section>
      )}

      {recurring.length > 0 && (
        <Section label={`recurring · ${recurring.length}`}>
          <ul className="flex flex-col gap-3">
            {recurring.map((item, index) => (
              <li key={index} className="flex items-baseline gap-3">
                <span
                  className="shrink-0 border border-sage-soft px-1.5 py-0.5 font-mono text-2xs text-sage"
                  title={item.rrule}
                >
                  ↻ {rruleLabel(item.rrule)}
                </span>
                <span className="text-sm leading-relaxed text-ink">{item.text}</span>
              </li>
            ))}
          </ul>
        </Section>
      )}

      {arbitrations.length > 0 && (
        <Section label={`arbitrations · ${arbitrations.length}`}>
          <ul className="flex flex-col gap-3">
            {arbitrations.map((arbitration, index) => (
              <li key={index}>
                <p className="text-sm leading-relaxed text-ink">{arbitration.statement}</p>
                <p className="mt-1 font-mono text-2xs text-ink-faint">
                  reconciled {arbitration.competing} competing{" "}
                  {arbitration.competing === 1 ? "entry" : "entries"}
                </p>
              </li>
            ))}
          </ul>
        </Section>
      )}
    </article>
  );
}

function EntryItem({
  entry,
  nameById,
  faded,
}: {
  entry: EntryView;
  nameById: Map<string, string>;
  faded?: boolean;
}) {
  const priv = isPrivate(entry.visibility);
  return (
    <li className={faded ? "opacity-55" : undefined}>
      <p
        className={
          "text-base leading-relaxed " + (faded ? "text-ink-soft line-through" : "text-ink")
        }
      >
        {entry.text}
      </p>
      <p className="mt-1 flex flex-wrap items-baseline gap-x-2.5 font-mono text-2xs text-ink-faint">
        <span>told by {tellerLabel(entry.told_by, nameById)}</span>
        <span className="text-ink-faint/45">·</span>
        <span className={priv ? "text-clay" : undefined}>
          {visibilityLabel(entry.visibility, nameById)}
        </span>
        <span className="text-ink-faint/45">·</span>
        <time dateTime={new Date(entry.asserted_at).toISOString()}>
          {formatDateTime(entry.asserted_at)}
        </time>
      </p>
    </li>
  );
}

function Section({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <section className="mt-6">
      <Eyebrow>{label}</Eyebrow>
      <div className="mt-3">{children}</div>
    </section>
  );
}

/// Group memories by their namespace prefix (`person/dave` → `person`), `self` standing alone, with
/// `self` first and the rest alphabetical — a stable, scannable order.
function groupByNamespace(memories: MemoryView[]): Array<[string, MemoryView[]]> {
  const namespaceOf = (name: string) => {
    const slash = name.indexOf("/");
    return slash === -1 ? name : name.slice(0, slash);
  };
  return groupBy(memories, (memory) => namespaceOf(memory.name)).sort(([a], [b]) => {
    if (a === "self") return -1;
    if (b === "self") return 1;
    return a.localeCompare(b);
  });
}

function leafName(name: string, namespace: string): string {
  return name === namespace ? name : name.slice(namespace.length + 1);
}
