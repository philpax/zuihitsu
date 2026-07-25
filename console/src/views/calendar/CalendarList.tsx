import type { AgendaItem } from "@zuihitsu/wire/wasm/console_wasm.js";
import type { Replica } from "../../lib/replica/replica.ts";
import { formatDate } from "../../lib/format/format.ts";
import { Link } from "../../lib/nav/history.tsx";
import { useStream } from "../../lib/nav/useStreamLocation.ts";
import { Eyebrow } from "../../components/primitives.tsx";
import { clockTime, groupByDayKey } from "./calendarUtilities.ts";

/// How far ahead recurring rules are projected in the list (they are unbounded, so they need a
/// horizon). One-off dated events have no such bound — every future one shows, even months out.
const HORIZON_DAYS = 60;

/// The upcoming list: the agent's horizon as a soonest-first agenda — every future dated one-off, plus
/// recurring rules projected `HORIZON_DAYS` ahead. The list face of the same occurrences the grids lay
/// out spatially; "now" is the agent's clock at the timeline cursor, so it time-travels with a scrub.
export function CalendarList({ replica, now }: { replica: Replica; now: number }) {
  const items = now > 0 ? replica.agenda(now, HORIZON_DAYS) : [];

  if (items.length === 0) {
    return (
      <p className="mx-auto max-w-prose py-12 text-center text-sm text-ink-faint">
        Nothing scheduled ahead.
      </p>
    );
  }

  const days = [...groupByDayKey(items)];
  return (
    <div>
      <div className="mb-6">
        <Eyebrow>{`all dated events · recurring ${HORIZON_DAYS} days out`}</Eyebrow>
      </div>
      <ol className="flex flex-col gap-6">
        {days.map(([key, dayItems]) => (
          <li key={key} className="grid grid-cols-1 gap-x-6 gap-y-2 sm:grid-cols-[9rem_1fr]">
            <div className="sm:sticky sm:top-4 sm:self-start">
              <p className="font-serif text-base text-ink">{formatDate(dayItems[0].when)}</p>
              <Eyebrow>{weekday(dayItems[0].when)}</Eyebrow>
            </div>
            <ul className="flex flex-col gap-3 border-l border-line pl-5">
              {dayItems.map((item, index) => (
                <ListRow key={index} item={item} />
              ))}
            </ul>
          </li>
        ))}
      </ol>
    </div>
  );
}

function ListRow({ item }: { item: AgendaItem }) {
  const stream = useStream();
  // A day-granular occurrence (and its noon sort) carries no stated time; only a precise instant does.
  const at = item.all_day ? null : clockTime(item.when);
  return (
    <li className="flex items-baseline gap-3">
      <span className="w-12 shrink-0 font-mono text-2xs text-ink-faint">{at ?? "—"}</span>
      <div className="min-w-0 flex-1">
        <p className="text-sm/relaxed text-ink">{item.text}</p>
        <p className="mt-0.5 flex items-baseline gap-2 font-mono text-2xs text-ink-faint">
          <Link
            to={stream.link.state(item.memory, { entry: item.entry, seq: stream.seq })}
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

function weekday(ms: number): string {
  return new Date(ms).toLocaleDateString("en-GB", { weekday: "long" });
}
