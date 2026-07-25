import type { ReactNode } from "react";

import type { EventCategory } from "../lib/model/events.ts";
import { CATEGORY_COLOR } from "../lib/model/events.ts";
import { formatDateTime, formatIso } from "../lib/format/format.ts";

/// The shared one-line log row: an optional seq column, the event type in its category colour, the
/// truncated summary, and a right-aligned ISO timestamp, expanding in place into whatever detail the
/// caller renders beneath. The Events and Background views both read the log through this row, so
/// the two stay one visual system; the seq column and the type-click filter are the Events view's
/// extras, switched by the props.
export function LogEventRow({
  seq,
  type,
  category,
  summary,
  recordedAt,
  open,
  onToggle,
  onTypeClick,
  children,
}: {
  /// Rendered as the leading right-aligned column when present; omitted entirely when absent.
  seq?: number;
  type: string;
  category: EventCategory;
  summary: string;
  recordedAt: number;
  open: boolean;
  onToggle: () => void;
  /// When present, clicking the type narrows a filter instead of toggling the row.
  onTypeClick?: () => void;
  /// The expanded detail, rendered when `open`.
  children: ReactNode;
}) {
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
        {onTypeClick ? (
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
        )}
        <span className={"truncate " + (open ? "text-ink" : "text-ink-soft")} title={summary}>
          {summary}
        </span>
        <time
          className="hidden shrink-0 text-right text-ink-faint sm:block"
          dateTime={new Date(recordedAt).toISOString()}
          title={formatDateTime(recordedAt)}
        >
          {formatIso(recordedAt)}
        </time>
      </button>
      {open && <div className="border-l-2 border-line py-3 pr-2 pl-4">{children}</div>}
    </div>
  );
}
