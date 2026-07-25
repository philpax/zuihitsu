import { useState } from "react";

import type { Replica } from "../../lib/replica/replica.ts";
import { DayEvent, GridNav } from "./calendarParts.tsx";
import {
  WEEKDAY_LABELS,
  dayKey,
  groupByDayKey,
  monthTitle,
  monthWeeks,
  rangeOf,
  shiftMonths,
} from "./calendarUtilities.ts";

/// How many occurrences a month cell shows before collapsing the rest into a "+N more" count — a
/// dense day would otherwise blow the cell's height out and break the grid's even rhythm.
const CELL_LIMIT = 3;

/// The classic month grid: full Monday-to-Sunday weeks covering the anchored month, each day a cell of
/// its occurrences. The visible month is component state — prev/today/next walk it backward and forward
/// without touching the URL — so the grid shows past, present, and future alike. Today wears a clay
/// accent; days spilling from the adjacent months are dimmed; recurring instances carry the ↻ glyph.
export function MonthGrid({ replica, now }: { replica: Replica; now: number }) {
  // Fall back to real "now" only when the log carries no clock yet (an empty stream); the lazy
  // initializer keeps that impure read out of render, running once at mount.
  const [anchor, setAnchor] = useState(() => (now > 0 ? now : Date.now()));
  const weeks = monthWeeks(anchor);
  const month = new Date(anchor).getMonth();
  const { start, end } = rangeOf(weeks.flat());
  const byDay = groupByDayKey(replica.occurrences(start, end));
  const todayKey = now > 0 ? dayKey(now) : null;

  return (
    <div>
      <GridNav
        title={monthTitle(anchor)}
        onPrev={() => setAnchor(shiftMonths(anchor, -1))}
        onToday={() => setAnchor(now > 0 ? now : Date.now())}
        onNext={() => setAnchor(shiftMonths(anchor, 1))}
      />
      <div className="overflow-x-auto">
        <div className="grid min-w-176 grid-cols-7 border-t border-l border-line">
          {WEEKDAY_LABELS.map((label) => (
            <div
              key={label}
              className="border-r border-b border-line bg-oat/30 px-2 py-1 text-center font-mono text-2xs tracking-widest text-ink-soft uppercase"
            >
              {label}
            </div>
          ))}
          {weeks.flat().map((day) => {
            const key = dayKey(day.getTime());
            const items = byDay.get(key) ?? [];
            const isToday = key === todayKey;
            const outside = day.getMonth() !== month;
            return (
              <div
                key={key}
                className={
                  "flex min-h-24 flex-col gap-0.5 border-r border-b border-line px-1.5 py-1 " +
                  (outside ? "bg-paper-raised/40" : "")
                }
              >
                <span
                  className={
                    "mb-0.5 self-end font-mono text-2xs " +
                    (isToday
                      ? "flex size-4 items-center justify-center rounded-full bg-clay font-medium text-paper"
                      : outside
                        ? "text-ink-faint/60"
                        : "text-ink-soft")
                  }
                >
                  {day.getDate()}
                </span>
                {items.slice(0, CELL_LIMIT).map((item, index) => (
                  <DayEvent key={index} item={item} />
                ))}
                {items.length > CELL_LIMIT && (
                  <span className="px-1 text-2xs text-ink-faint">
                    +{items.length - CELL_LIMIT} more
                  </span>
                )}
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}
