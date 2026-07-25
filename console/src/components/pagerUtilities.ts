/// The shared list-paging constants and slicing, split from the Pager component so the component
/// file exports only components (the fast-refresh boundary rule). One page size everywhere so the
/// log-reading views feel like one system.

export const PAGE_SIZE = 100;

/// Slice one page out of an already-sorted list.
export function pageOf<T>(items: readonly T[], page: number): T[] {
  return items.slice(page * PAGE_SIZE, (page + 1) * PAGE_SIZE);
}

/// How many page numbers show on each side of the current page.
export const PAGE_WINDOW_RADIUS = 3;

/// The forum-style page list over 0-based pages: a window around the current page with the first
/// and last pages always anchored, and `null` marking an elided gap. The list keeps a constant
/// `2 * radius + 5` entries whenever that many pages exist — the run pads near the ends instead of
/// shrinking — so the control does not change width as the current page moves.
export function buildPageList(page: number, pages: number, radius: number): (number | null)[] {
  const slots = 2 * radius + 5;
  const range = (from: number, to: number): number[] =>
    Array.from({ length: to - from + 1 }, (_, i) => from + i);
  if (pages <= slots) return range(0, pages - 1);
  if (page <= radius + 1) return [...range(0, slots - 3), null, pages - 1];
  if (page >= pages - radius - 2) return [0, null, ...range(pages - (slots - 2), pages - 1)];
  return [0, null, ...range(page - radius, page + radius), null, pages - 1];
}
