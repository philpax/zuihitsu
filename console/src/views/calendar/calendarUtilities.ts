import type { AgendaItem } from "@zuihitsu/wire/wasm/console_wasm.js";

/// The three faces of the Calendar view, riding the URL selection segment. The month grid is the
/// default (a bare `/live/calendar` opens it); the list is the former upcoming-agenda behaviour.
export type CalendarMode = "list" | "week" | "month";

export const CALENDAR_MODES: ReadonlyArray<{ id: CalendarMode; label: string }> = [
  { id: "month", label: "Month" },
  { id: "week", label: "Week" },
  { id: "list", label: "List" },
];

export const DEFAULT_MODE: CalendarMode = "month";

/// Narrow a raw URL selection segment to a calendar mode, defaulting to the month grid when the
/// segment is absent or names no mode — so a bare URL and a stale deep link both resolve rather than
/// stranding the view.
export function asCalendarMode(value: string | null): CalendarMode {
  return CALENDAR_MODES.some((mode) => mode.id === value) ? (value as CalendarMode) : DEFAULT_MODE;
}

/// The weekday column headers, Monday first — the week the grids lay out.
export const WEEKDAY_LABELS = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"] as const;

/// Local midnight of the day containing `ms`.
export function startOfDay(ms: number): Date {
  const date = new Date(ms);
  return new Date(date.getFullYear(), date.getMonth(), date.getDate());
}

/// A day's stable local key (`year-month-day`), for bucketing occurrences and matching "today". Built
/// from the local calendar fields, not an ISO slice, so it never drifts a day under a timezone offset.
export function dayKey(ms: number): string {
  const date = new Date(ms);
  return `${date.getFullYear()}-${date.getMonth() + 1}-${date.getDate()}`;
}

/// A new date `n` days after `date` (negative to go back), via the local calendar so it stays correct
/// across a daylight-saving boundary.
export function addDays(date: Date, n: number): Date {
  return new Date(date.getFullYear(), date.getMonth(), date.getDate() + n);
}

/// Local midnight of the Monday beginning the week that contains `ms` — weeks start Monday.
export function mondayOf(ms: number): Date {
  const day = startOfDay(ms);
  // getDay is 0 (Sunday) through 6 (Saturday); shift so Monday is 0.
  const offset = (day.getDay() + 6) % 7;
  return addDays(day, -offset);
}

/// The seven days of the week containing `anchorMs`, Monday first.
export function weekDays(anchorMs: number): Date[] {
  const monday = mondayOf(anchorMs);
  return Array.from({ length: 7 }, (_, index) => addDays(monday, index));
}

/// The weeks of the month grid for the month containing `anchorMs`: full Monday-to-Sunday rows from
/// the week holding the first of the month through the week holding the last, so leading and trailing
/// days spill from the adjacent months (rendered dimmed by the grid).
export function monthWeeks(anchorMs: number): Date[][] {
  const anchor = new Date(anchorMs);
  const first = new Date(anchor.getFullYear(), anchor.getMonth(), 1);
  const last = new Date(anchor.getFullYear(), anchor.getMonth() + 1, 0);
  const weeks: Date[][] = [];
  for (
    let weekStart = mondayOf(first.getTime());
    weekStart.getTime() <= last.getTime();
    weekStart = addDays(weekStart, 7)
  ) {
    weeks.push(Array.from({ length: 7 }, (_, index) => addDays(weekStart, index)));
  }
  return weeks;
}

/// The half-open `[start, end)` millisecond window covering a run of days — the range a grid hands the
/// replica to expand occurrences over.
export function rangeOf(days: Date[]): { start: number; end: number } {
  return { start: days[0].getTime(), end: addDays(days[days.length - 1], 1).getTime() };
}

/// Shift a month anchor by `delta` whole months, landing on the first of the target month.
export function shiftMonths(anchorMs: number, delta: number): number {
  const date = new Date(anchorMs);
  return new Date(date.getFullYear(), date.getMonth() + delta, 1).getTime();
}

/// Shift a day anchor by `delta` days, keeping the weekday (the week grid steps by seven).
export function shiftDays(anchorMs: number, delta: number): number {
  return addDays(new Date(anchorMs), delta).getTime();
}

/// Bucket occurrences (already soonest-first) by their local day key, preserving order within a day.
export function groupByDayKey(items: AgendaItem[]): Map<string, AgendaItem[]> {
  const groups = new Map<string, AgendaItem[]>();
  for (const item of items) {
    const key = dayKey(item.when);
    const bucket = groups.get(key);
    if (bucket) bucket.push(item);
    else groups.set(key, [item]);
  }
  return groups;
}

/// "July 2026" — the month grid's title.
export function monthTitle(anchorMs: number): string {
  return new Date(anchorMs).toLocaleDateString("en-GB", { month: "long", year: "numeric" });
}

/// "30 Jun – 6 Jul 2026" — the week grid's title, spanning the row.
export function weekTitle(days: Date[]): string {
  const label = (date: Date) =>
    date.toLocaleDateString("en-GB", { day: "numeric", month: "short" });
  return `${label(days[0])} – ${label(days[days.length - 1])} ${days[days.length - 1].getFullYear()}`;
}

/// The wall-clock time of a precise instant, or `null` when it lands exactly on local midnight (shown
/// without a misleading "00:00"). Day-granular occurrences are gated out by the caller on `all_day`, so
/// a day reference never leaks its noon sort as a time.
export function clockTime(ms: number): string | null {
  const date = new Date(ms);
  if (date.getHours() === 0 && date.getMinutes() === 0) return null;
  return date.toLocaleTimeString("en-GB", { hour: "2-digit", minute: "2-digit" });
}
