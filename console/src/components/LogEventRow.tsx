import type { ReactNode } from "react";

import type { EventCategory } from "../lib/model/events.ts";
import { CATEGORY_COLOR } from "../lib/model/events.ts";
import { formatDateTime, formatIso } from "../lib/format/format.ts";

/// The shared one-line event row: an optional seq column, the event type in its category colour, the
/// truncated summary, and an optional right-aligned ISO timestamp, expanding in place into whatever
/// detail the caller renders beneath. Every expandable event row reads through this component so they
/// stay one visual system: the Events and Background views render the full log geometry (seq column
/// and type-click filter are the Events view's extras), while a turn's outcome trail in the
/// Conversation view renders the `compact` inline variant. The variations are switched by the props.
export function LogEventRow({
  seq,
  type,
  category,
  summary,
  recordedAt,
  open,
  onToggle,
  onTypeClick,
  compact = false,
  children,
}: {
  /// Rendered as the leading right-aligned column when present; omitted entirely when absent.
  seq?: number;
  type: string;
  category: EventCategory;
  summary: string;
  /// The event's wall-clock time, shown right-aligned in its own column. Omitted entirely when
  /// absent — a turn's inline outcome rows drop it for a compact reading.
  recordedAt?: number;
  open: boolean;
  onToggle: () => void;
  /// When present, clicking the type narrows a filter instead of toggling the row.
  onTypeClick?: () => void;
  /// The compact inline variant a turn's outcome trail uses: a leading ↳ marker in place of the seq
  /// and time columns, no row separator, and tighter spacing. The default is the full log geometry.
  compact?: boolean;
  /// The expanded detail, rendered when `open`.
  children: ReactNode;
}) {
  const typeEl = onTypeClick ? (
    <span
      // Click the type to narrow to just it — a precise filter under the coarse categories.
      // The row is a button, so this stays a span and stops the toggle.
      role="button"
      tabIndex={-1}
      onClick={(click) => {
        click.stopPropagation();
        onTypeClick();
      }}
      className={"truncate hover:underline " + CATEGORY_COLOR[category]}
      title={`Filter to ${type}`}
    >
      {type}
    </span>
  ) : (
    <span className={"truncate " + CATEGORY_COLOR[category]}>{type}</span>
  );

  if (compact) {
    return (
      <div className="font-mono text-xs">
        <button
          onClick={onToggle}
          title={open ? "Collapse" : "Expand the event"}
          className="group flex w-full items-baseline gap-2 text-left"
        >
          <span className="text-ink-faint">↳</span>
          {typeEl}
          <span
            className={
              "truncate transition-colors " +
              (open ? "text-ink" : "text-ink-soft group-hover:text-ink")
            }
            title={summary}
          >
            {summary}
          </span>
        </button>
        {open && <div className="my-1 ml-4 border-l-2 border-line py-1 pl-3">{children}</div>}
      </div>
    );
  }

  const columns =
    seq !== undefined
      ? "grid-cols-[2.25rem_7rem_1fr] sm:grid-cols-[3rem_11rem_1fr_auto]"
      : "grid-cols-[7rem_1fr] sm:grid-cols-[11rem_1fr_auto]";
  return (
    <div className="border-b border-line/60 font-mono text-xs">
      <button
        onClick={onToggle}
        title={open ? "Collapse" : "Expand the event"}
        className={`grid w-full items-baseline gap-3 py-2 text-left sm:gap-4 ${columns}`}
      >
        {seq !== undefined && (
          <span className={"text-right " + (open ? "text-clay" : "text-ink-faint")}>{seq}</span>
        )}
        {typeEl}
        <span className={"truncate " + (open ? "text-ink" : "text-ink-soft")} title={summary}>
          {summary}
        </span>
        {recordedAt !== undefined && (
          <time
            className="hidden shrink-0 text-right text-ink-faint sm:block"
            dateTime={new Date(recordedAt).toISOString()}
            title={formatDateTime(recordedAt)}
          >
            {formatIso(recordedAt)}
          </time>
        )}
      </button>
      {open && <div className="border-l-2 border-line py-3 pr-2 pl-4">{children}</div>}
    </div>
  );
}
