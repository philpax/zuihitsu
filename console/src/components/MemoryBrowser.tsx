import { useState } from "react";

import type { Replica } from "../lib/replica.ts";
import type { EntryView, MemoryDetail, MemoryView } from "../lib/graph.ts";
import { isPrivate, tellerLabel, visibilityLabel } from "../lib/labels.ts";
import { Eyebrow } from "./primitives.tsx";

/// The two-pane memory browser shared by the State and Time-travel views: a namespace-grouped list
/// on the left, the opened memory's contents, links, and `same_as` class on the right. Selection is
/// controlled by the parent so it survives the remount the Time-travel scrubber forces on each fold.
/// The console sees everything — superseded entries and all visibilities, plainly marked.
export function MemoryBrowser({
  replica,
  selected,
  onSelect,
}: {
  replica: Replica;
  selected: string | null;
  onSelect: (name: string) => void;
}) {
  const memories = replica.memories("");
  const nameById = new Map(memories.map((memory) => [memory.id, memory.name]));
  const [query, setQuery] = useState("");

  if (memories.length === 0) {
    return (
      <div className="py-24 text-center text-sm text-ink-faint">
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
    <div className="grid grid-cols-[15rem_1fr] gap-12">
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
          <MemoryList memories={listed} selected={effective} onSelect={onSelect} />
        )}
      </div>
      {detail ? (
        <MemoryDetailPane detail={detail} nameById={nameById} />
      ) : (
        <div className="py-24 text-center text-sm text-ink-faint">Select a memory.</div>
      )}
    </div>
  );
}

function MemoryList({
  memories,
  selected,
  onSelect,
}: {
  memories: MemoryView[];
  selected: string | null;
  onSelect: (name: string) => void;
}) {
  const groups = groupByNamespace(memories);

  return (
    <nav className="flex flex-col gap-6 self-start">
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
                    className={
                      "-ml-3 flex w-full items-baseline border-l-2 py-1 pl-2.5 text-left font-mono text-xs transition-colors " +
                      (active
                        ? "border-clay text-ink"
                        : "border-transparent text-ink-soft hover:text-ink")
                    }
                  >
                    {leafName(memory.name, namespace)}
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
}: {
  detail: MemoryDetail;
  nameById: Map<string, string>;
}) {
  const { memory, entries, history, links } = detail;
  const superseded = history.filter((entry) => entry.superseded_by !== null);
  const classPeers = detail.class.filter((peer) => peer.id !== memory.id);

  return (
    <article className="max-w-prose">
      <header className="border-b border-line pb-5">
        <div className="flex items-baseline justify-between gap-4">
          <h2 className="font-mono text-base text-ink">{memory.name}</h2>
          <Eyebrow>{memory.volatility} volatility</Eyebrow>
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
      </p>
    </li>
  );
}

function Section({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <section className="mt-8">
      <Eyebrow>{label}</Eyebrow>
      <div className="mt-3">{children}</div>
    </section>
  );
}

/// Group memories by their namespace prefix (`person/dave` → `person`), `self` standing alone, with
/// `self` first and the rest alphabetical — a stable, scannable order.
function groupByNamespace(memories: MemoryView[]): Array<[string, MemoryView[]]> {
  const groups = new Map<string, MemoryView[]>();
  for (const memory of memories) {
    const slash = memory.name.indexOf("/");
    const namespace = slash === -1 ? memory.name : memory.name.slice(0, slash);
    const bucket = groups.get(namespace);
    if (bucket) bucket.push(memory);
    else groups.set(namespace, [memory]);
  }
  return [...groups.entries()].sort(([a], [b]) => {
    if (a === "self") return -1;
    if (b === "self") return 1;
    return a.localeCompare(b);
  });
}

function leafName(name: string, namespace: string): string {
  return name === namespace ? name : name.slice(namespace.length + 1);
}
