/// The render window over a conversation's already-folded turns array — a Discord-style transcript
/// renders only a bounded slice of turns rather than the whole thousand-turn history at once. The fold
/// itself stays whole (the replica holds every event); this is purely a *view* window, a half-open
/// index range `[start, end)` into `conversation.turns`, that the sentinels page in and out as the
/// reader scrolls. Every function here is pure over the range and the turn count, so the windowing
/// arithmetic is unit-tested without a scroll harness (`transcriptWindowUtilities.test.ts`).

/// How many turns the tail opens with, and the span a deep-linked window centres around its focus.
export const INITIAL_WINDOW_TURNS = 50;
/// How many turns a single page-in adds (or a page-out drops) when a sentinel crosses the viewport.
export const CHUNK_TURNS = 25;
/// The most turns the window ever holds at once, so the DOM stays bounded no matter how far the reader
/// pages: growing one end past this trims the other, the end furthest from the viewport.
export const MAX_WINDOW_TURNS = 100;

/// A half-open index range `[start, end)` into the flat, seq-ordered turns array.
export interface TurnWindow {
  start: number;
  end: number;
}

/// The window a conversation opens at: the last [`INITIAL_WINDOW_TURNS`] (the tail), or — when a deep
/// link names a turn — a span centred on that turn so the linked moment lands mid-window with history
/// either side to scroll into.
export function initialWindow(total: number, focusIndex: number | null): TurnWindow {
  if (total <= 0) return { start: 0, end: 0 };
  if (focusIndex === null) return tailWindow(total);
  const start = Math.max(0, focusIndex - CHUNK_TURNS);
  const end = Math.min(total, focusIndex + CHUNK_TURNS + 1);
  return { start, end };
}

/// The tail window — the newest [`INITIAL_WINDOW_TURNS`] turns — where a jump-to-latest lands and a
/// pinned reader follows.
export function tailWindow(total: number): TurnWindow {
  return { start: Math.max(0, total - INITIAL_WINDOW_TURNS), end: Math.max(0, total) };
}

/// Extend the window backwards by a chunk (the reader scrolled to the top of the loaded range). If the
/// widened span would exceed [`MAX_WINDOW_TURNS`], trim the tail end — the end furthest from the
/// viewport, which the reader has scrolled away from — so the DOM stays bounded.
export function pageInStart(win: TurnWindow, max = MAX_WINDOW_TURNS): TurnWindow {
  const start = Math.max(0, win.start - CHUNK_TURNS);
  const end = Math.min(win.end, start + max);
  return { start, end };
}

/// Extend the window forwards by a chunk toward the tail (the reader scrolled back down). If the
/// widened span would exceed [`MAX_WINDOW_TURNS`], trim the head — the end the reader has scrolled away
/// from — so the DOM stays bounded.
export function pageInEnd(win: TurnWindow, total: number, max = MAX_WINDOW_TURNS): TurnWindow {
  const end = Math.min(total, win.end + CHUNK_TURNS);
  const start = Math.max(win.start, end - max);
  return { start, end };
}

/// Snap the window to the tail as new turns land, for a reader pinned at the foot: the end tracks the
/// latest turn and the head is bounded to [`MAX_WINDOW_TURNS`] behind it, paging out the oldest loaded
/// turns (the reader is at the bottom, not reading them).
export function followWindow(total: number, max = MAX_WINDOW_TURNS): TurnWindow {
  return { start: Math.max(0, total - max), end: Math.max(0, total) };
}

/// Clamp a window to a turn count that may have shrunk — the timeline cursor scrubbed back filters the
/// turns array down, so a window captured at a later horizon must be pulled back into range.
export function clampWindow(win: TurnWindow, total: number): TurnWindow {
  const end = Math.min(win.end, total);
  const start = Math.min(win.start, end);
  return { start: Math.max(0, start), end: Math.max(0, end) };
}

/// How many turns sit past the window's end — the tail the reader has scrolled away from, which the
/// jump-to-latest indicator counts as "new since you left the foot".
export function unseenTailCount(win: TurnWindow, total: number): number {
  return Math.max(0, total - win.end);
}
