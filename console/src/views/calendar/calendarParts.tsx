import type { ReactNode } from "react";

import type { AgendaItem } from "@zuihitsu/wire/wasm/console_wasm.js";
import { Link } from "../../lib/nav/history.tsx";
import { useStream } from "../../lib/nav/useStreamLocation.ts";
import { clockTime } from "./calendarUtilities.ts";

// The pieces the week and month grids share: a single occurrence rendered inside a day cell, and the
// prev/today/next chrome above a grid. Both live here rather than in one grid so neither owns the
// other's copy, mirroring the shared-reference-family pattern in `components/eventDetailParts.tsx`.

/// One occurrence inside a day cell: a truncated label linking to its dated entry in the State view (per
/// the console's no-bare-ids rule), the ↻ glyph in sage for a recurring instance, and — in the
/// roomier week grid (`showTime`) — a leading clock time for a precise instant. The full text, memory,
/// and time ride the `title`, since the label itself is clipped. The calendar renders only inside a
/// stream frame, so the State link always resolves.
export function DayEvent({ item, showTime = false }: { item: AgendaItem; showTime?: boolean }) {
  const stream = useStream();
  const at = showTime && !item.all_day ? clockTime(item.when) : null;
  const title = `${item.text} · ${item.memory}${at ? ` · ${at}` : ""}${
    item.recurring ? " · recurring" : ""
  }`;
  return (
    <Link
      to={stream.link.state(item.memory, { entry: item.entry, seq: stream.seq })}
      title={title}
      className="flex items-baseline gap-1 rounded-xs px-1 py-0.5 text-2xs/tight text-ink-soft transition-colors hover:bg-oat/50 hover:text-ink"
    >
      {at && <span className="shrink-0 font-mono text-ink-faint">{at}</span>}
      {item.recurring && (
        <span className="shrink-0 text-sage" title="recurring" aria-label="recurring">
          ↻
        </span>
      )}
      <span className="truncate">{item.text}</span>
    </Link>
  );
}

/// The chrome above a grid: its title (the month, or the week's date span) and the prev/today/next
/// controls that walk the visible range backward and forward. The range itself is the grid's own
/// component state, so these controls do not touch the URL.
export function GridNav({
  title,
  onPrev,
  onToday,
  onNext,
}: {
  title: string;
  onPrev: () => void;
  onToday: () => void;
  onNext: () => void;
}) {
  return (
    <div className="mb-4 flex items-center justify-between gap-4">
      <h2 className="font-serif text-lg text-ink">{title}</h2>
      <div className="flex items-center gap-1 font-mono text-2xs">
        <NavButton onClick={onPrev} title="previous">
          ‹
        </NavButton>
        <button
          onClick={onToday}
          className="flex h-7 items-center rounded-xs border border-line-strong px-2.5 tracking-wide text-ink-soft uppercase transition-colors hover:border-clay hover:text-clay"
        >
          today
        </button>
        <NavButton onClick={onNext} title="next">
          ›
        </NavButton>
      </div>
    </div>
  );
}

function NavButton({
  onClick,
  title,
  children,
}: {
  onClick: () => void;
  title: string;
  children: ReactNode;
}) {
  return (
    <button
      onClick={onClick}
      title={title}
      className="flex h-7 items-center rounded-xs border border-line-strong px-2.5 text-sm text-ink-soft transition-colors hover:border-clay hover:text-clay"
    >
      {children}
    </button>
  );
}
