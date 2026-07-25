/// The shared list pager: forum-style numbered pages with the first and last anchored, an elision
/// gap on each side of the moving window, and prev/next arrows — for the log-reading views that
/// window a long newest-first list (events, background events, relations). The page size, slicing,
/// and page-list construction live in pagerUtilities.ts, keeping this file component-only for fast
/// refresh.

import { PAGE_SIZE, PAGE_WINDOW_RADIUS, buildPageList } from "./pagerUtilities.ts";

export function Pager({
  page,
  total,
  onPage,
}: {
  page: number;
  total: number;
  onPage: (page: number) => void;
}) {
  const pages = Math.max(1, Math.ceil(total / PAGE_SIZE));
  if (pages <= 1) return null;
  const clamped = Math.min(page, pages - 1);
  return (
    <nav className="mt-6 flex items-baseline gap-1.5 font-mono text-xs">
      <button
        onClick={() => onPage(clamped - 1)}
        disabled={clamped === 0}
        aria-label="previous page"
        className="pr-1 text-ink-soft transition-colors hover:text-ink disabled:cursor-default disabled:text-ink-faint/60"
      >
        ←
      </button>
      {buildPageList(clamped, pages, PAGE_WINDOW_RADIUS).map((p, i) =>
        p === null ? (
          <span key={`gap-${i}`} className="px-0.5 text-ink-faint">
            …
          </span>
        ) : (
          <button
            key={p}
            onClick={() => onPage(p)}
            aria-current={p === clamped ? "page" : undefined}
            className={
              "min-w-6 px-1 text-center transition-colors " +
              (p === clamped ? "border-b border-clay text-ink" : "text-ink-soft hover:text-ink")
            }
          >
            {p + 1}
          </button>
        ),
      )}
      <button
        onClick={() => onPage(clamped + 1)}
        disabled={clamped >= pages - 1}
        aria-label="next page"
        className="pl-1 text-ink-soft transition-colors hover:text-ink disabled:cursor-default disabled:text-ink-faint/60"
      >
        →
      </button>
      {/* Keyed by the current page so an outside page change re-seeds the box — uncontrolled, so
          typing needs no state, and commit happens on Enter or blur. */}
      <input
        key={clamped}
        defaultValue={clamped + 1}
        inputMode="numeric"
        aria-label="go to page"
        title="Go to page"
        onKeyDown={(event) => {
          if (event.key === "Enter") event.currentTarget.blur();
        }}
        onBlur={(event) => {
          const parsed = Number.parseInt(event.currentTarget.value, 10);
          if (Number.isFinite(parsed)) {
            onPage(Math.min(pages, Math.max(1, parsed)) - 1);
          } else {
            event.currentTarget.value = String(clamped + 1);
          }
        }}
        className="ml-2 w-9 border-b border-line bg-transparent pb-0.5 text-center text-ink-soft focus:border-ink-faint focus:text-ink focus:outline-none"
      />
      <span className="ml-auto text-ink-faint">{total} total</span>
    </nav>
  );
}
