import type { EntryView, MemoryView } from "../../lib/model/graph.ts";
import type { RecurringItem } from "../../lib/model/audit.ts";
import { isPrivate, tellerLabel, visibilityLabel } from "../../lib/model/labels.ts";
import { formatDateTime } from "../../lib/format/format.ts";
import { Eyebrow } from "../../components/primitives.tsx";
import { groupByNamespace, leafName } from "./memoryUtilities.ts";

function MemoryRef({ name, onSelect }: { name: string; onSelect: (name: string) => void }) {
  return (
    <button
      onClick={() => onSelect(name)}
      title={`Open ${name}`}
      className="text-clay underline-offset-2 transition-colors hover:text-ink hover:underline"
    >
      {name}
    </button>
  );
}

export function MemoryList({
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
                    title={
                      memory.description ? `${memory.name} — ${memory.description}` : memory.name
                    }
                    className={
                      "-ml-3 flex w-full min-w-0 flex-col border-l-2 py-1 pl-2.5 text-left transition-colors " +
                      (active
                        ? "border-clay text-ink"
                        : "border-transparent text-ink-soft hover:text-ink")
                    }
                  >
                    <span className="flex w-full min-w-0 items-baseline font-mono text-xs">
                      <span className="truncate">{leafName(memory.name, namespace)}</span>
                      {recurring.has(memory.id) && (
                        <span className="ml-1.5 shrink-0 text-sage" title="recurring">
                          ↻
                        </span>
                      )}
                    </span>
                    {/* The synthesized description, clamped, so the list reads as a glanceable index of
                        what each memory is about rather than a bare list of names. */}
                    {memory.description && (
                      <span className="mt-0.5 line-clamp-2 text-2xs leading-snug text-ink-faint">
                        {memory.description}
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

export function EntryItem({
  entry,
  nameById,
  faded,
  disputed,
}: {
  entry: EntryView;
  nameById: Map<string, string>;
  faded?: boolean;
  disputed?: boolean;
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
        {entry.retracted_reason !== null && (
          <>
            <span className="text-clay">retracted: {entry.retracted_reason}</span>
            <span className="text-ink-faint/45">·</span>
          </>
        )}
        {disputed && (
          <>
            <span className="text-clay">disputed</span>
            <span className="text-ink-faint/45">·</span>
          </>
        )}
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

export function Section({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <section className="mt-6">
      <Eyebrow>{label}</Eyebrow>
      <div className="mt-3">{children}</div>
    </section>
  );
}

export { MemoryRef };
