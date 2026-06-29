import { Link } from "react-router-dom";

import type { Event } from "../types/Event.ts";
import type { AgendaItem } from "../lib/graph.ts";
import type { Replica } from "../lib/replica.ts";
import { formatDate } from "../lib/format.ts";
import { statePath } from "../lib/routes.ts";
import { useStreamBase } from "../lib/useStreamLocation.ts";
import { Eyebrow } from "../components/primitives.tsx";

/// How far ahead recurring rules are projected (they are unbounded, so they need a horizon). One-off
/// dated events have no such bound — every future one shows, even months out.
const HORIZON_DAYS = 60;

/// The Agenda view: the agent's horizon — its upcoming dated and recurring events, soonest first.
/// The other face of the recurring data the State view marks per memory; here the same occurrences
/// (plus one-offs) are projected forward and laid out as a calendar. "Now" is the agent's clock at
/// the timeline cursor — the head when following live, the run's end for a finished package — so the
/// agenda is what the agent saw ahead of it at that moment, and it time-travels with a scrub.
export function AgendaView({
  replica,
  events,
  cursor,
}: {
  replica: Replica;
  events: Event[];
  cursor: number;
}) {
  const base = useStreamBase();
  const now = events.reduce(
    (max, event) => (event.seq <= cursor && event.recorded_at > max ? event.recorded_at : max),
    0,
  );
  const items = now > 0 ? replica.agenda(now, HORIZON_DAYS) : [];

  if (items.length === 0) {
    return (
      <div>
        <div className="mb-6">
          <Eyebrow>{now > 0 ? "no events ahead" : "no events yet"}</Eyebrow>
        </div>
        <p className="mx-auto max-w-prose py-12 text-center text-sm text-ink-faint">
          Nothing scheduled ahead.
        </p>
      </div>
    );
  }

  const days = groupByDay(items);
  return (
    <div>
      <div className="mb-6">
        <Eyebrow>
          {`as of ${formatDate(now)} · all dated events · recurring ${HORIZON_DAYS} days out`}
        </Eyebrow>
      </div>
      <ol className="flex flex-col gap-6">
        {days.map(([day, dayItems]) => (
          <li key={day} className="grid grid-cols-1 gap-x-6 gap-y-2 sm:grid-cols-[9rem_1fr]">
            <div className="sm:sticky sm:top-4 sm:self-start">
              <p className="font-serif text-base text-ink">{day}</p>
              <Eyebrow>{weekday(dayItems[0].when)}</Eyebrow>
            </div>
            <ul className="flex flex-col gap-3 border-l border-line pl-5">
              {dayItems.map((item, index) => (
                <AgendaRow key={index} item={item} base={base} cursor={cursor} />
              ))}
            </ul>
          </li>
        ))}
      </ol>
    </div>
  );
}

function AgendaRow({ item, base, cursor }: { item: AgendaItem; base: string; cursor: number }) {
  // A day-granular occurrence (and its noon sort) carries no stated time; only a precise instant does.
  const at = item.all_day ? null : clockTime(item.when);
  return (
    <li className="flex items-baseline gap-3">
      <span className="w-12 shrink-0 font-mono text-2xs text-ink-faint">{at ?? "—"}</span>
      <div className="min-w-0 flex-1">
        <p className="text-sm leading-relaxed text-ink">{item.text}</p>
        <p className="mt-0.5 flex items-baseline gap-2 font-mono text-2xs text-ink-faint">
          <Link
            to={statePath(base, cursor, item.memory)}
            title={`Open ${item.memory} in State`}
            className="truncate text-clay underline-offset-2 transition-colors hover:text-ink hover:underline"
          >
            {item.memory}
          </Link>
          {item.recurring && (
            <span className="shrink-0 text-sage" title="recurring">
              ↻
            </span>
          )}
        </p>
      </div>
    </li>
  );
}

/// Group items (already soonest-first) by calendar day, preserving order.
function groupByDay(items: AgendaItem[]): Array<[string, AgendaItem[]]> {
  const groups = new Map<string, AgendaItem[]>();
  for (const item of items) {
    const day = formatDate(item.when);
    const bucket = groups.get(day);
    if (bucket) bucket.push(item);
    else groups.set(day, [item]);
  }
  return [...groups.entries()];
}

function weekday(ms: number): string {
  return new Date(ms).toLocaleDateString("en-GB", { weekday: "long" });
}

/// The wall-clock time of a precise instant, or `null` when it lands exactly on local midnight (shown
/// without a misleading "00:00"). Day-granular occurrences never reach here — the caller gates those
/// on `all_day`, so a day reference no longer leaks its noon sort as a time.
function clockTime(ms: number): string | null {
  const date = new Date(ms);
  if (date.getHours() === 0 && date.getMinutes() === 0) return null;
  return date.toLocaleTimeString("en-GB", { hour: "2-digit", minute: "2-digit" });
}
