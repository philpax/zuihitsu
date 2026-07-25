import { useState } from "react";

import type { Replica } from "../../lib/replica/replica.ts";
import { DayEvent, GridNav } from "./calendarParts.tsx";
import {
  WEEKDAY_LABELS,
  dayKey,
  groupByDayKey,
  rangeOf,
  shiftDays,
  weekDays,
  weekTitle,
} from "./calendarUtilities.ts";

/// The seven-day week grid: one tall column per day, Monday first, each listing that day's occurrences
/// in full (the week affords more room than the month's cells, so every occurrence shows with its clock
/// time). The visible week is component state — prev/today/next step it by seven days without touching
/// the URL. Today's column header wears a clay accent; recurring instances carry the ↻ glyph.
export function WeekGrid({ replica, now }: { replica: Replica; now: number }) {
  // Fall back to real "now" only when the log carries no clock yet (an empty stream); the lazy
  // initializer keeps that impure read out of render, running once at mount.
  const [anchor, setAnchor] = useState(() => (now > 0 ? now : Date.now()));
  const days = weekDays(anchor);
  const { start, end } = rangeOf(days);
  const byDay = groupByDayKey(replica.occurrences(start, end));
  const todayKey = now > 0 ? dayKey(now) : null;

  return (
    <div>
      <GridNav
        title={weekTitle(days)}
        onPrev={() => setAnchor(shiftDays(anchor, -7))}
        onToday={() => setAnchor(now > 0 ? now : Date.now())}
        onNext={() => setAnchor(shiftDays(anchor, 7))}
      />
      <div className="overflow-x-auto">
        <div className="grid min-w-208 grid-cols-7 border-t border-l border-line">
          {days.map((day, index) => {
            const key = dayKey(day.getTime());
            const items = byDay.get(key) ?? [];
            const isToday = key === todayKey;
            return (
              <div key={key} className="flex min-h-48 flex-col border-r border-line">
                <div
                  className={
                    "flex items-baseline justify-between gap-1 border-b border-line px-2 py-1.5 " +
                    (isToday ? "bg-clay/10" : "bg-oat/30")
                  }
                >
                  <span className="font-mono text-2xs tracking-widest text-ink-soft uppercase">
                    {WEEKDAY_LABELS[index]}
                  </span>
                  <span
                    className={
                      "font-mono text-xs " +
                      (isToday
                        ? "flex size-5 items-center justify-center rounded-full bg-clay font-medium text-paper"
                        : "text-ink")
                    }
                  >
                    {day.getDate()}
                  </span>
                </div>
                <div className="flex flex-col gap-0.5 px-1 py-1.5">
                  {items.length === 0 ? (
                    <span className="px-1 text-2xs text-ink-faint/60">—</span>
                  ) : (
                    items.map((item, itemIndex) => (
                      <DayEvent key={itemIndex} item={item} showTime />
                    ))
                  )}
                </div>
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}
