import { useState } from "react";

import type { Event } from "../types/Event.ts";
import type { Replica } from "../lib/replica.ts";
import { type EventCategory, CATEGORY_COLOR, eventCategory, eventSummary } from "../lib/events.ts";
import { nameById } from "../lib/labels.ts";
import { Eyebrow } from "../components/primitives.tsx";
import { EventDetail } from "./EventDetail.tsx";

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
  const [active, setActive] = useState<Set<EventCategory>>(() => new Set(CATEGORIES));
  const [search, setSearch] = useState("");
  const [expanded, setExpanded] = useState<number | null>(null);

  const needle = search.trim().toLowerCase();
  const rows = events
    .filter((event) => event.seq <= cursor)
    .map((event) => ({
      event,
      category: eventCategory(event.payload.type),
      summary: eventSummary(event.payload, names),
    }))
    .filter(({ event, category, summary }) => {
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

  return (
    <section>
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

      <div className="mb-3 flex items-baseline justify-between">
        <Eyebrow>{rows.length} events</Eyebrow>
        <Eyebrow>
          seq 1 – {cursor}
          {cursor < events.length ? ` of ${events.length}` : ""}
        </Eyebrow>
      </div>

      <ul className="font-mono text-xs">
        {rows.map(({ event, category, summary }) => {
          const open = expanded === event.seq;
          return (
            <li key={event.seq} className="border-b border-line/60">
              <button
                onClick={() => setExpanded(open ? null : event.seq)}
                className="grid w-full grid-cols-[2.25rem_7rem_1fr] items-baseline gap-3 py-2 text-left sm:grid-cols-[3rem_11rem_1fr] sm:gap-4"
              >
                <span className={"text-right " + (open ? "text-clay" : "text-ink-faint")}>
                  {event.seq}
                </span>
                <span className={"truncate " + CATEGORY_COLOR[category]} title={event.payload.type}>
                  {event.payload.type}
                </span>
                <span
                  className={"truncate " + (open ? "text-ink" : "text-ink-soft")}
                  title={summary}
                >
                  {summary}
                </span>
              </button>
              {open && (
                <div className="border-l-2 border-line py-3 pl-4 pr-2">
                  <EventDetail payload={event.payload} nameById={names} />
                </div>
              )}
            </li>
          );
        })}
      </ul>
    </section>
  );
}
