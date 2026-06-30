import { useEffect, useState } from "react";

import type { Event } from "../types/Event.ts";
import { Replica } from "../lib/replica.ts";
import { type MemoryChange, diffSnapshots, snapshotAt } from "../lib/diff.ts";
import { eventSummary } from "../lib/events.ts";
import { Eyebrow } from "../components/primitives.tsx";

/// The Time-travel view: diff the materialized graph between two points in the log. The global
/// scrubber sets one end (the cursor); a baseline slider here sets the other, and the two are compared
/// forward (earlier → later) regardless of order. It folds its own private replica so the comparison
/// never disturbs the shared timeline the other views read. What changed is read off the graph — which
/// memories were added or removed, and which gained entries, a new description, tags, or a rename —
/// with the events that bridged the two points listed beneath.
export function DiffView({
  events,
  cursor,
  head,
}: {
  events: Event[];
  cursor: number;
  head: number;
}) {
  const [base, setBase] = useState<Replica | null>(null);
  const [from, setFrom] = useState(0);
  const [changes, setChanges] = useState<MemoryChange[] | null>(null);

  // A private replica for the diff — folded freely here without touching the shared timeline fold.
  useEffect(() => {
    let cancelled = false;
    Replica.fromEvents(events).then((replica) => !cancelled && setBase(replica));
    return () => {
      cancelled = true;
    };
  }, [events]);

  const lo = Math.min(from, cursor);
  const hi = Math.max(from, cursor);

  // Folding the wasm replica is imperative work that can't run during render, so the diff is computed
  // in an effect. Deferred a microtask off the synchronous tick so the fold/compare is scheduled work
  // rather than a synchronous setState cascade (and cancellable if the inputs change first).
  useEffect(() => {
    if (!base) return;
    let cancelled = false;
    void Promise.resolve().then(() => {
      if (cancelled) return;
      const before = snapshotAt(base, lo);
      const after = snapshotAt(base, hi);
      setChanges(diffSnapshots(before, after));
    });
    return () => {
      cancelled = true;
    };
  }, [base, lo, hi]);

  // The events that bridge the two points, narrated — the raw moves behind the state change.
  const nameById = new Map<string, string>();
  for (const event of events) {
    if (event.payload.type === "MemoryCreated") nameById.set(event.payload.id, event.payload.name);
    else if (event.payload.type === "MemoryRenamed")
      nameById.set(event.payload.id, event.payload.new_name);
  }
  const bridging = events.filter((event) => event.seq > lo && event.seq <= hi);

  return (
    <div>
      <div className="mb-6">
        <div className="mb-1.5 flex items-baseline justify-between gap-4">
          <Eyebrow>baseline</Eyebrow>
          <span className="font-mono text-2xs text-ink-soft">
            comparing seq {lo} → {hi}
            {lo === hi && " · pick two different points"}
          </span>
        </div>
        <input
          type="range"
          min={0}
          max={head}
          value={from}
          onChange={(event) => setFrom(Number(event.target.value))}
          className="w-full accent-clay"
        />
      </div>

      {changes === null ? (
        <p className="py-7 text-center text-sm text-ink-faint">Folding…</p>
      ) : changes.length === 0 ? (
        <p className="py-7 text-center text-sm text-ink-faint">
          {lo === hi ? "Move either end to compare." : "No graph changes between these points."}
        </p>
      ) : (
        <ul className="flex flex-col">
          {changes.map((change) => (
            <li
              key={change.id}
              className="flex items-baseline gap-3 border-b border-line py-2.5 last:border-b-0"
            >
              <Sigil kind={change.kind} />
              <span className="min-w-0 flex-1 truncate font-mono text-sm text-ink">
                {change.name}
              </span>
              <span className="shrink-0 text-right font-mono text-2xs text-ink-soft">
                {describe(change)}
              </span>
            </li>
          ))}
        </ul>
      )}

      {bridging.length > 0 && (
        <section className="mt-8 border-t border-line pt-6">
          <Eyebrow>events between</Eyebrow>
          <ul className="mt-3 flex flex-col gap-1">
            {bridging.map((event) => (
              <li key={event.seq} className="flex items-baseline gap-2 font-mono text-2xs">
                <span className="shrink-0 text-ink-faint">{event.seq}</span>
                <span className="shrink-0 text-ink-soft">{event.payload.type}</span>
                <span className="truncate text-ink-faint">
                  {eventSummary(event.payload, nameById)}
                </span>
              </li>
            ))}
          </ul>
        </section>
      )}
    </div>
  );
}

/// A one-glyph marker for the change kind, in the kind's accent.
function Sigil({ kind }: { kind: MemoryChange["kind"] }) {
  const glyph = kind === "added" ? "+" : kind === "removed" ? "−" : "~";
  const tone = kind === "added" ? "text-sage" : kind === "removed" ? "text-clay" : "text-ink-soft";
  return <span className={"shrink-0 font-mono text-sm " + tone}>{glyph}</span>;
}

/// The right-hand readout: for a change, the fields that differ (with the signed entry delta); for a
/// presence change, the entry count that came or went.
function describe(change: MemoryChange): string {
  if (change.kind === "added")
    return change.entryDelta > 0 ? `new · ${entries(change.entryDelta)}` : "new";
  if (change.kind === "removed") return "removed";
  return change.fields
    .map((field) => (field === "entries" ? signedEntries(change.entryDelta) : field))
    .join(", ");
}

function entries(n: number): string {
  return `${n} ${n === 1 ? "entry" : "entries"}`;
}

function signedEntries(delta: number): string {
  return `${delta > 0 ? "+" : "−"}${entries(Math.abs(delta))}`;
}
