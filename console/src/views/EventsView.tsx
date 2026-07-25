import { useState } from "react";

import type { Event } from "@zuihitsu/wire/types/Event.ts";
import type { EventSource } from "@zuihitsu/wire/types/EventSource.ts";
import type { Replica } from "../lib/replica/replica.ts";
import type { StepRecord } from "@zuihitsu/wire/types/StepRecord.ts";
import {
  type EventCategory,
  CATEGORY_COLOR,
  EVENT_SOURCES,
  eventCategory,
  eventSummary,
  eventTouchesMemory,
  sourceLabel,
} from "../lib/model/events.ts";
import { buildStepMarkers, type StepMarker } from "../lib/model/stepJournal.ts";
import { nameById } from "../lib/model/labels.ts";
import { useStream } from "../lib/nav/useStreamLocation.ts";
import { LogEventRow } from "../components/LogEventRow.tsx";
import { Pager } from "../components/Pager.tsx";
import { pageOf, PAGE_SIZE } from "../components/pagerUtilities.ts";
import { Eyebrow } from "../components/primitives.tsx";
import { EventDetail } from "../components/EventDetail.tsx";
import { conversationNameById } from "../lib/model/conversationNameById.ts";

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
/// exactly these rows. Newest events sit at the top, and the list is paged at [`PAGE_SIZE`] so a
/// thousands-row log stays a plain, compiler-memoised render rather than a virtualised window. An
/// eval run also carries its step journal, which draws a hairline boundary above the first event of
/// each scenario beat; a live tail has no journal, so the stream is unbroken.
export function EventsView({
  replica,
  events,
  cursor,
  journal,
  resumedFromStep,
}: {
  replica: Replica;
  events: Event[];
  cursor: number;
  journal?: readonly StepRecord[];
  resumedFromStep?: number | null;
}) {
  const names = nameById(replica.memories(""));
  const convNames = conversationNameById(replica.conversations());
  const { search: streamSearch, patchSearch } = useStream();
  // The memory the view is pinned to (the State view's "events touching this" jump), carried in the
  // URL so the focus is shareable and survives back/forward. `null` shows the whole log.
  const focusId = streamSearch.focus ?? null;
  const focusName = focusId ? (names.get(focusId) ?? focusId) : null;
  const [active, setActive] = useState<Set<EventCategory>>(() => new Set(CATEGORIES));
  const [activeSources, setActiveSources] = useState<Set<EventSource>>(
    () => new Set(EVENT_SOURCES),
  );
  const [search, setSearch] = useState("");
  const [typeFilter, setTypeFilter] = useState<string | null>(null);
  const [expanded, setExpanded] = useState<number | null>(null);
  // The current page into the newest-first list. Any filter change resets it to the first page, so a
  // narrower result set is never left scrolled past its own end.
  const [page, setPage] = useState(0);

  function clearFocus() {
    patchSearch((prev) => ({ ...prev, focus: undefined }));
    setPage(0);
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
      if (!activeSources.has(event.source)) return false;
      if (!needle) return true;
      return (
        event.payload.type.toLowerCase().includes(needle) || summary.toLowerCase().includes(needle)
      );
    })
    // Newest first: sort by recorded time descending, falling back to seq so events sharing a
    // timestamp keep a stable, log-order-reversed sequence.
    .sort((a, b) => b.event.recorded_at - a.event.recorded_at || b.event.seq - a.event.seq);

  // The deep-link anchor: an `?event=<seq>` names one event to page to and expand — the target of an
  // `EntryRef` link from another view. When the seq is among the filtered rows, page to it and expand
  // it once per distinct target (keyed on the seq, the guarded render-adjust pattern), so the operator
  // can page or collapse freely afterwards. A seq the active filters exclude degrades silently: no
  // state change, so the guard simply keeps looking should a filter later reveal it.
  const targetEvent = streamSearch.event != null ? Number(streamSearch.event) : null;
  const [anchoredEvent, setAnchoredEvent] = useState<number | null>(null);
  if (targetEvent !== null && targetEvent !== anchoredEvent) {
    const index = rows.findIndex((row) => row.event.seq === targetEvent);
    if (index >= 0) {
      setAnchoredEvent(targetEvent);
      setPage(Math.floor(index / PAGE_SIZE));
      setExpanded(targetEvent);
    }
  }

  const pageCount = Math.max(1, Math.ceil(rows.length / PAGE_SIZE));
  const clampedPage = Math.min(page, pageCount - 1);
  const pageRows = pageOf(rows, clampedPage);

  // The step boundaries, keyed by the seq they sit above. Anchored against the full log (the first
  // event carries the genesis marker), so a boundary shows wherever its anchor event survives the
  // active filters. Empty for a live tail or an old package without a journal.
  const stepMarkers = buildStepMarkers(
    journal ?? [],
    events[0]?.seq ?? null,
    resumedFromStep ?? null,
  );

  function toggle(category: EventCategory) {
    const next = new Set(active);
    if (next.has(category)) next.delete(category);
    else next.add(category);
    setActive(next);
    setPage(0);
  }

  function toggleSource(source: EventSource) {
    const next = new Set(activeSources);
    if (next.has(source)) next.delete(source);
    else next.add(source);
    setActiveSources(next);
    setPage(0);
  }

  function changeSearch(value: string) {
    setSearch(value);
    setPage(0);
  }

  function changeTypeFilter(next: string | null) {
    setTypeFilter(next);
    setPage(0);
  }

  return (
    <section>
      {focusName && (
        <div className="mb-5 flex items-baseline justify-between gap-4 border-l-2 border-clay bg-clay-soft/15 py-2 pr-2 pl-3">
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
      {/* The labels sit in their own column, the member lists on a shared vertical baseline well
          clear of them — so "type" and "by" read as row headers, not as members of the lists. */}
      <div className="mb-7 grid grid-cols-[auto_1fr] items-baseline gap-x-8 gap-y-1">
        <span className="font-mono text-2xs tracking-widest text-ink-faint uppercase">type</span>
        <div className="flex flex-col gap-4 sm:flex-row sm:items-baseline sm:justify-between sm:gap-6">
          <div className="flex flex-wrap gap-x-4 gap-y-2">
            {CATEGORIES.map((category) => (
              <button
                key={category}
                onClick={() => toggle(category)}
                className={
                  "font-mono text-2xs tracking-widest uppercase transition-colors " +
                  (active.has(category)
                    ? CATEGORY_COLOR[category]
                    : "text-ink-faint/45 line-through")
                }
              >
                {category}
              </button>
            ))}
          </div>
          <input
            value={search}
            onChange={(event) => changeSearch(event.target.value)}
            placeholder="filter…"
            className="w-full border-b border-line bg-transparent pb-1 font-mono text-xs text-ink placeholder:text-ink-faint/60 focus:border-ink-faint focus:outline-none sm:w-44"
          />
        </div>
        <span className="font-mono text-2xs tracking-widest text-ink-faint uppercase">by</span>
        <div className="flex flex-wrap gap-x-4 gap-y-2">
          {EVENT_SOURCES.map((source) => (
            <button
              key={sourceLabel(source)}
              onClick={() => toggleSource(source)}
              className={
                "font-mono text-2xs tracking-widest uppercase transition-colors " +
                (activeSources.has(source) ? "text-ink-soft" : "text-ink-faint/45 line-through")
              }
              title={`Filter to events authored by the ${sourceLabel(source)}`}
            >
              {sourceLabel(source)}
            </button>
          ))}
        </div>
      </div>

      <div className="mb-3 flex items-baseline justify-between gap-4">
        <div className="flex items-baseline gap-3">
          <Eyebrow>{rows.length} events</Eyebrow>
          {typeFilter && (
            <button
              onClick={() => changeTypeFilter(null)}
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

      <div className="font-mono text-xs">
        {pageRows.map(({ event, category, summary }) => {
          const open = expanded === event.seq;
          const markers = stepMarkers.get(event.seq);
          return (
            <div key={event.seq}>
              {markers && <StepBoundary markers={markers} />}
              <LogEventRow
                seq={event.seq}
                type={event.payload.type}
                category={category}
                summary={summary}
                recordedAt={event.recorded_at}
                open={open}
                onToggle={() => setExpanded(open ? null : event.seq)}
                onTypeClick={() =>
                  changeTypeFilter(typeFilter === event.payload.type ? null : event.payload.type)
                }
              >
                <EventDetail
                  payload={event.payload}
                  nameById={names}
                  conversationNameById={convNames}
                  seq={event.seq}
                  recordedAt={event.recorded_at}
                  source={event.source}
                />
              </LogEventRow>
            </div>
          );
        })}
      </div>

      <Pager page={clampedPage} total={rows.length} onPage={setPage} />
    </section>
  );
}

/// A step boundary drawn above the first event of a scenario beat: a hairline rule carrying the step's
/// index and one-line summary. The `genesis` marker precedes the birth events, and a resumed run's
/// `resume` note — the one piece of replay state the trace needs — marks in clay where the live
/// continuation takes over from the restored recording. Metadata in faint ink, not a loud header.
function StepBoundary({ markers }: { markers: StepMarker[] }) {
  return (
    <div className="mt-4 flex flex-col gap-1 border-t border-line pt-2">
      {markers.map((marker, index) =>
        marker.kind === "genesis" ? (
          <span key={index} className="font-mono text-2xs tracking-widest text-ink-faint uppercase">
            genesis
          </span>
        ) : marker.kind === "resume" ? (
          <span key={index} className="font-mono text-2xs text-clay" title="resumed run boundary">
            resumed here — steps above are the restored recording
          </span>
        ) : (
          <span key={index} className="flex items-baseline gap-2">
            <span className="shrink-0 font-mono text-2xs tracking-widest text-ink-faint uppercase">
              step {marker.index}
            </span>
            <span className="truncate font-mono text-2xs text-ink-soft" title={marker.label}>
              {marker.label}
            </span>
            {marker.skipped && (
              <span className="shrink-0 font-mono text-2xs text-ink-faint italic">skipped</span>
            )}
          </span>
        ),
      )}
    </div>
  );
}
