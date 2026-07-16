import type { CSSProperties } from "react";

import type { Event } from "@zuihitsu/wire/types/Event.ts";
import { eventSummary } from "../lib/model/events.ts";
import { formatDate, formatDateTime, formatTime } from "../lib/format/format.ts";

/// The global time cursor for the run-scoped views: a sticky scrubber over the run's seq range that
/// every view reflects — the State graph folds to it, the Conversation and Events stop there. The
/// event at the cursor narrates the move; the readout doubles as a reset to the head.
export function Timeline({
  head,
  seq,
  events,
  onScrub,
  onReset,
}: {
  head: number;
  seq: number;
  events: Event[];
  onScrub: (seq: number) => void;
  onReset: () => void;
}) {
  const current = events.find((event) => event.seq === seq) ?? null;
  const nameById = namesUpTo(events, seq);
  const atHead = seq >= head;
  // The run's span, for the dates flanking the scrubber (events arrive in seq order).
  const first = events[0] ?? null;
  const last = events[events.length - 1] ?? null;

  return (
    <div className="border-t border-line py-2.5">
      <div className="mb-1 flex min-w-0 items-baseline gap-2 font-mono text-2xs">
        <button
          onClick={onReset}
          className={"shrink-0 " + (atHead ? "text-ink-faint" : "text-clay hover:text-ink")}
          title={atHead ? "At the latest state" : "Jump to the latest state"}
        >
          seq {seq} / {head}
        </button>
        {current && (
          <>
            <time
              className="shrink-0 text-ink-faint"
              dateTime={new Date(current.recorded_at).toISOString()}
              title={formatDateTime(current.recorded_at)}
            >
              · {formatTime(current.recorded_at)}
            </time>
            <span className="truncate text-ink-faint">
              · {current.payload.type} {eventSummary(current.payload, nameById)}
            </span>
          </>
        )}
      </div>
      {/* The scrubber, flanked by single-step buttons: each nudges the cursor one event, for a precise
          walk where dragging the range is too coarse. Stepping onto the head re-enters follow mode. */}
      <div className="flex items-center gap-2">
        <StepButton
          label="←"
          title="Step back one event"
          onClick={() => onScrub(Math.max(0, seq - 1))}
          disabled={seq <= 0}
        />
        <input
          type="range"
          min={0}
          max={head}
          value={seq}
          onChange={(event) => onScrub(Number(event.target.value))}
          className="scrubber min-w-0 flex-1"
          style={{ "--scrubbed": `${head > 0 ? (seq / head) * 100 : 0}%` } as CSSProperties}
        />
        <StepButton
          label="→"
          title="Step forward one event"
          onClick={() => onScrub(Math.min(head, seq + 1))}
          disabled={seq >= head}
        />
      </div>
      {/* The run's span flanks the scrubber where there is room; on a phone the bottom chrome
          stays two rows. */}
      {first && last && (
        <div className="mt-1 hidden justify-between font-mono text-2xs text-ink-faint/70 sm:flex">
          <span>{formatDate(first.recorded_at)}</span>
          <span>{formatDate(last.recorded_at)}</span>
        </div>
      )}
    </div>
  );
}

/// One single-step arrow flanking the scrubber — a faint mono glyph that dims when the cursor is
/// already at that end of the range.
function StepButton({
  label,
  title,
  onClick,
  disabled,
}: {
  label: string;
  title: string;
  onClick: () => void;
  disabled: boolean;
}) {
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      title={title}
      className="shrink-0 px-1 font-mono text-xs text-ink-faint transition-colors hover:text-ink disabled:cursor-default disabled:text-ink-faint/40 disabled:hover:text-ink-faint/40"
    >
      {label}
    </button>
  );
}

/// The id → handle map as it stood at `seq`, built from the create and rename events up to that
/// point — enough to name the event at the cursor without depending on the current fold.
function namesUpTo(events: Event[], seq: number): Map<string, string> {
  const names = new Map<string, string>();
  for (const event of events) {
    if (event.seq > seq) break;
    if (event.payload.type === "MemoryCreated") names.set(event.payload.id, event.payload.name);
    else if (event.payload.type === "MemoryRenamed")
      names.set(event.payload.id, event.payload.new_name);
  }
  return names;
}
