import type { EntryId } from "@zuihitsu/wire/types/EntryId.ts";
import type { MemoryDetail } from "../../lib/model/graph.ts";
import type { Arbitration, RecurringItem } from "../../lib/model/audit.ts";
import { formatDateTime } from "../../lib/format/format.ts";
import { rruleLabel } from "../../lib/model/audit.ts";
import { Eyebrow } from "../../components/primitives.tsx";
import { EntryItem, MemoryRef, Section } from "./MemoryList.tsx";
import { SelfEditor } from "./SelfEditor.tsx";

export function MemoryDetailPane({
  detail,
  nameById,
  arbitrations,
  recurring,
  onShowEvents,
  onSelect,
  onEditSelf,
  onRetract,
}: {
  detail: MemoryDetail;
  nameById: Map<string, string>;
  arbitrations: Arbitration[];
  recurring: RecurringItem[];
  onShowEvents?: (id: string, name: string) => void;
  onSelect: (name: string) => void;
  /// Present only in the live agent frame at the head, and exercised only on `self`: append a charter
  /// entry, or revise one under operator authority (the operator side of self-editing).
  onEditSelf?: (text: string, supersedes?: EntryId) => Promise<void>;
  /// Retract a live entry under operator authority. Present only in the live agent frame at the head.
  onRetract?: (memory: string, entry: EntryId, reason: string) => Promise<void>;
}) {
  const { memory, entries, history, links } = detail;
  // A retraction tombstones an entry with its own id in superseded_by and a reason; a plain
  // supersession points at a distinct successor. Split them so each reads as what it is.
  const retracted = history.filter((entry) => entry.retracted_reason !== null);
  const superseded = history.filter(
    (entry) => entry.superseded_by !== null && entry.retracted_reason === null,
  );
  const classPeers = detail.class.filter((peer) => peer.id !== memory.id);
  const disputed = new Set(detail.disputed);

  return (
    <article className="max-w-[46rem]">
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
            same as{" "}
            {classPeers.map((peer, index) => (
              <span key={peer.id}>
                {index > 0 && ", "}
                <MemoryRef name={peer.name} onSelect={onSelect} />
              </span>
            ))}
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
              <EntryItem
                key={entry.entry_id}
                entry={entry}
                nameById={nameById}
                disputed={disputed.has(entry.entry_id)}
                memoryName={memory.name}
                onRetract={onRetract}
              />
            ))}
          </ul>
        )}
      </Section>

      {memory.name === "self" && onEditSelf && (
        <SelfEditor entries={entries} onEditSelf={onEditSelf} />
      )}

      {links.length > 0 && (
        <Section label={`links · ${links.length}`}>
          <ul className="flex flex-col gap-1.5 font-mono text-xs text-ink-soft">
            {links.map((link, index) => {
              const target = nameById.get(link.to);
              return (
                <li key={index} className="flex items-baseline gap-2">
                  <span className="text-clay">{link.relation}</span>
                  <span className="text-ink-faint">→</span>
                  {target ? (
                    <MemoryRef name={target} onSelect={onSelect} />
                  ) : (
                    <span>{link.to}</span>
                  )}
                </li>
              );
            })}
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

      {retracted.length > 0 && (
        <Section label={`retracted · ${retracted.length}`}>
          <ul className="flex flex-col gap-4">
            {retracted.map((entry) => (
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
