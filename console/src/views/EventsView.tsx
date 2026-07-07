import { useLayoutEffect, useRef, useState } from "react";
import { useSearchParams } from "react-router-dom";
import { useWindowVirtualizer } from "@tanstack/react-virtual";

import type { Event } from "../types/Event.ts";
import type { Replica } from "../lib/replica/replica.ts";
import {
  type EventCategory,
  CATEGORY_COLOR,
  eventCategory,
  eventSummary,
  eventTouchesMemory,
} from "../lib/model/events.ts";
import { nameById } from "../lib/model/labels.ts";
import { formatDateTime, formatTime } from "../lib/format/format.ts";
import { useStreamBase } from "../lib/nav/useStreamLocation.ts";
import { Eyebrow } from "../components/primitives.tsx";
import { EventDetail } from "../components/EventDetail.tsx";

const CATEGORIES: EventCategory[] = [
  "memory",
  "link",
  "conversation",
  "deliberation",
  "lifecycle",
  "infra",
];

/// The Events view: the run's log as the source of truth, filtered by category and free text, and
/// stopped at the timeline cursor. A flat, scannable stream — every other view is a projection of
/// exactly these rows.
export function EventsView({
  replica,
  events,
  cursor,
}: {
  replica: Replica;
  events: Event[];
  cursor: number;
}) {
  const names = nameById(replica.memories(""));
  const base = useStreamBase();
  const [searchParams, setSearchParams] = useSearchParams();
  // The memory the view is pinned to (the State view's "events touching this" jump), carried in the
  // URL so the focus is shareable and survives back/forward. `null` shows the whole log.
  const focusId = searchParams.get("focus");
  const focusName = focusId ? (names.get(focusId) ?? focusId) : null;
  const [active, setActive] = useState<Set<EventCategory>>(() => new Set(CATEGORIES));
  const [search, setSearch] = useState("");
  const [typeFilter, setTypeFilter] = useState<string | null>(null);
  const [expanded, setExpanded] = useState<number | null>(null);

  function clearFocus() {
    setSearchParams(
      (prev) => {
        const updated = new URLSearchParams(prev);
        updated.delete("focus");
        return updated;
      },
      { replace: true },
    );
  }

  const needle = search.trim().toLowerCase();
  const rows = events
    .filter((event) => event.seq <= cursor)
    .map((event) => ({
      event,
      category: eventCategory(event.payload.type),
      summary: eventSummary(event.payload, names),
    }))
    .filter(({ event, category, summary }) => {
      if (focusId && !eventTouchesMemory(event.payload, focusId)) return false;
      if (typeFilter && event.payload.type !== typeFilter) return false;
      if (!active.has(category)) return false;
      if (!needle) return true;
      return (
        event.payload.type.toLowerCase().includes(needle) || summary.toLowerCase().includes(needle)
      );
    });

  function toggle(category: EventCategory) {
    const next = new Set(active);
    if (next.has(category)) next.delete(category);
    else next.add(category);
    setActive(next);
  }

  // A run's log is thousands of rows, so only the visible window is rendered. The list scrolls with
  // the page (window virtualizer), so `scrollMargin` tracks the list's offset from the top of the
  // document — re-measured when the layout above it changes (the verdict panel collapsing, a resize).
  // Rows measure their own height, so an expanded row's detail is accounted for without a fixed size.
  const listRef = useRef<HTMLDivElement>(null);
  const [scrollMargin, setScrollMargin] = useState(0);
  useLayoutEffect(() => {
    const measure = () => listRef.current && setScrollMargin(listRef.current.offsetTop);
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(document.body);
    return () => observer.disconnect();
  }, []);
  const virtualizer = useWindowVirtualizer({
    count: rows.length,
    estimateSize: () => 37,
    overscan: 12,
    scrollMargin,
  });

  return (
    <section>
      {focusName && (
        <div className="mb-5 flex items-baseline justify-between gap-4 border-l-2 border-clay bg-clay-soft/15 py-2 pl-3 pr-2">
          <span className="font-mono text-xs text-ink-soft">
            events touching <span className="text-ink">{focusName}</span>
          </span>
          <button
            onClick={clearFocus}
            className="shrink-0 font-mono text-xs text-clay transition-colors hover:text-ink"
          >
            clear ✕
          </button>
        </div>
      )}
      <div className="mb-7 flex flex-col gap-4 sm:flex-row sm:items-center sm:justify-between sm:gap-6">
        <div className="flex flex-wrap gap-x-4 gap-y-2">
          {CATEGORIES.map((category) => (
            <button
              key={category}
              onClick={() => toggle(category)}
              className={
                "font-mono text-2xs uppercase tracking-widest transition-colors " +
                (active.has(category) ? CATEGORY_COLOR[category] : "text-ink-faint/45 line-through")
              }
            >
              {category}
            </button>
          ))}
        </div>
        <input
          value={search}
          onChange={(event) => setSearch(event.target.value)}
          placeholder="filter…"
          className="w-full border-b border-line bg-transparent pb-1 font-mono text-xs text-ink placeholder:text-ink-faint/60 focus:border-ink-faint focus:outline-none sm:w-44"
        />
      </div>

      <div className="mb-3 flex items-baseline justify-between gap-4">
        <div className="flex items-baseline gap-3">
          <Eyebrow>{rows.length} events</Eyebrow>
          {typeFilter && (
            <button
              onClick={() => setTypeFilter(null)}
              className="font-mono text-xs text-clay transition-colors hover:text-ink"
              title="Clear the type filter"
            >
              {typeFilter} ✕
            </button>
          )}
        </div>
        <Eyebrow>
          seq 1 – {cursor}
          {cursor < events.length ? ` of ${events.length}` : ""}
        </Eyebrow>
      </div>

      <div ref={listRef} className="font-mono text-xs">
        <div className="relative" style={{ height: `${virtualizer.getTotalSize()}px` }}>
          {virtualizer.getVirtualItems().map((item) => {
            const { event, category, summary } = rows[item.index];
            const open = expanded === event.seq;
            return (
              <div
                key={event.seq}
                data-index={item.index}
                ref={virtualizer.measureElement}
                className="absolute left-0 top-0 w-full border-b border-line/60"
                style={{
                  transform: `translateY(${item.start - virtualizer.options.scrollMargin}px)`,
                }}
              >
                <button
                  onClick={() => setExpanded(open ? null : event.seq)}
                  className="grid w-full grid-cols-[2.25rem_7rem_1fr] items-baseline gap-3 py-2 text-left sm:grid-cols-[3rem_11rem_1fr_auto] sm:gap-4"
                >
                  <span className={"text-right " + (open ? "text-clay" : "text-ink-faint")}>
                    {event.seq}
                  </span>
                  <span
                    // Click the type to narrow to just it — a precise filter under the coarse
                    // categories. The row is a button, so this stays a span and stops the toggle.
                    role="button"
                    tabIndex={-1}
                    onClick={(click) => {
                      click.stopPropagation();
                      setTypeFilter((current) =>
                        current === event.payload.type ? null : event.payload.type,
                      );
                    }}
                    className={"truncate hover:underline " + CATEGORY_COLOR[category]}
                    title={`Filter to ${event.payload.type}`}
                  >
                    {event.payload.type}
                  </span>
                  <span
                    className={"truncate " + (open ? "text-ink" : "text-ink-soft")}
                    title={summary}
                  >
                    {summary}
                  </span>
                  <time
                    className="hidden shrink-0 text-right text-ink-faint sm:block"
                    dateTime={new Date(event.recorded_at).toISOString()}
                    title={formatDateTime(event.recorded_at)}
                  >
                    {formatTime(event.recorded_at)}
                  </time>
                </button>
                {open && (
                  <div className="border-l-2 border-line py-3 pl-4 pr-2">
                    <EventDetail
                      payload={event.payload}
                      nameById={names}
                      base={base}
                      seq={event.seq}
                      recordedAt={event.recorded_at}
                    />
                  </div>
                )}
              </div>
            );
          })}
        </div>
      </div>
    </section>
  );
}
