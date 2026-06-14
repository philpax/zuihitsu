import type { Event } from "../types/Event.ts";
import type { AgendaItem } from "../lib/graph.ts";
import type { Replica } from "../lib/replica.ts";
import { formatDate } from "../lib/format.ts";
import { Eyebrow } from "../components/primitives.tsx";

/// How far ahead the agenda looks. Recurring rules are projected to their next instance within this
/// window, so a weekly standup always appears even when no one-off event is near.
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
  const now = events.reduce(
    (max, event) => (event.seq <= cursor && event.recorded_at > max ? event.recorded_at : max),
    0,
  );
  const items = now > 0 ? replica.agenda(now, HORIZON_DAYS) : [];

  if (items.length === 0) {
    return (
      <div className="mx-auto max-w-prose">
        <Header now={now} />
        <p className="py-16 text-center text-sm text-ink-faint">
          Nothing on the horizon in the next {HORIZON_DAYS} days.
        </p>
      </div>
    );
  }

  const days = groupByDay(items);
  return (
    <div className="mx-auto max-w-prose">
      <Header now={now} />
      <ol className="flex flex-col gap-8">
        {days.map(([day, dayItems]) => (
          <li key={day} className="grid grid-cols-1 gap-x-8 gap-y-2 sm:grid-cols-[9rem_1fr]">
            <div className="sm:sticky sm:top-4 sm:self-start">
              <p className="font-serif text-base text-ink">{day}</p>
              <Eyebrow>{weekday(dayItems[0].when)}</Eyebrow>
            </div>
            <ul className="flex flex-col gap-3 border-l border-line pl-5">
              {dayItems.map((item, index) => (
                <AgendaRow key={index} item={item} />
              ))}
            </ul>
          </li>
        ))}
      </ol>
    </div>
  );
}

function Header({ now }: { now: number }) {
  return (
    <header className="mb-8">
      <h2 className="font-serif text-xl text-ink sm:text-2xl">Agenda</h2>
      <p className="mt-1 font-mono text-2xs uppercase tracking-widest text-ink-faint">
        {now > 0 ? `as of ${formatDate(now)} · next ${HORIZON_DAYS} days` : "no events yet"}
      </p>
    </header>
  );
}

function AgendaRow({ item }: { item: AgendaItem }) {
  const at = clockTime(item.when);
  return (
    <li className="flex items-baseline gap-3">
      <span className="w-12 shrink-0 font-mono text-2xs text-ink-faint">{at ?? "—"}</span>
      <div className="min-w-0 flex-1">
        <p className="text-sm leading-relaxed text-ink">{item.text}</p>
        <p className="mt-0.5 flex items-baseline gap-2 font-mono text-2xs text-ink-faint">
          <span className="truncate">{item.memory}</span>
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

/// The wall-clock time, or `null` for a day-granularity occurrence that lands on midnight (shown
/// without a time rather than a misleading "00:00").
function clockTime(ms: number): string | null {
  const date = new Date(ms);
  if (date.getHours() === 0 && date.getMinutes() === 0) return null;
  return date.toLocaleTimeString("en-GB", { hour: "2-digit", minute: "2-digit" });
}
