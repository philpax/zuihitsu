import type { ViewId } from "./streamViews.ts";
import type { StreamSearch } from "../../router.tsx";

/// The link builders for a stream's deep views, so every link builds the same shapes from one place.
/// Each returns a [`To`] descriptor — a pathname plus the typed search it carries — to spread into a
/// TanStack `<Link {...} />` or hand to `navigate(...)`. The route *tree* (in `router.tsx`) is the
/// single source of truth for the shapes; these builders address it. They stay string-based because
/// the stream views render under both the eval (`/eval/$scenario/$run/…`) and live (`/live/…`)
/// subtrees off a runtime `base`, so a shared view cannot know which typed route it is under.
///
/// Convention: **route segments** carry navigational identity — the view and its selection (the open
/// memory, room, or settings tab) — and are what back and forward move between, so selecting one
/// `push`es. **Search** carries filter and display state — the cursor (`seq`), a turn highlight
/// (`turn`), the Events pin (`focus`), the Relations filters — with `replace` for continuous
/// interaction and `push` for a discrete deep link. A free-form segment (a memory, a room key) is
/// encoded so it stays a single segment.
///
/// A [`To`]'s `search` is a *value*, so TanStack navigation with it **replaces** the whole search —
/// exactly what a cross-view jump wants (the source view's `turn`/`focus`/relation filters drop away).
/// Preserving-and-mutating one key instead (the cursor scrubber, a filter toggle) is the *function*
/// form — `navigate({ to: ".", search: (prev) => ({ ...prev, … }) })` — not one of these builders.
export interface To {
  to: string;
  search?: StreamSearch;
}

/// A run's path without a view — the prefix the view and cursor hang off.
export function runBase(scenario: string, run: number): string {
  return `/eval/${encodeURIComponent(scenario)}/${run}`;
}

/// A plain view path — no selection, only the cursor when pinned. For a tab switch or an eval-rail
/// link, where `view` is a runtime value rather than a fixed destination.
export function viewPath(base: string, view: ViewId, seq?: number | null): To {
  return { to: `${base}/${view}`, search: seqSearch(seq) };
}

/// A run opened at a particular view (the conversation by default — the payoff view).
export function runPath(scenario: string, run: number, view: ViewId = "conversation"): To {
  return viewPath(runBase(scenario, run), view);
}

/// The State view opened on a memory, folded to `seq` — the target an event's memory ref jumps to.
export function statePath(base: string, memory: string, seq?: number | null): To {
  return { to: `${base}/state/${encodeURIComponent(memory)}`, search: seqSearch(seq) };
}

/// The Conversation view, on a room when given (else the room is resolved from `turn`), optionally
/// highlighting a turn.
export function conversationPath(
  base: string,
  opts: { room?: string; turn?: string; seq?: number | null } = {},
): To {
  const room = opts.room ? `/${encodeURIComponent(opts.room)}` : "";
  return {
    to: `${base}/conversation${room}`,
    search: { ...(opts.turn ? { turn: opts.turn } : {}), ...seqSearch(opts.seq) },
  };
}

/// The Events view, pinned to a memory's events when given.
export function eventsPath(base: string, opts: { focus?: string; seq?: number | null } = {}): To {
  return {
    to: `${base}/events`,
    search: { ...(opts.focus ? { focus: opts.focus } : {}), ...seqSearch(opts.seq) },
  };
}

/// The Settings view opened on a tab.
export function settingsPath(base: string, section: string): To {
  return { to: `${base}/settings/${section}` };
}

/// The cursor as a search fragment, or empty when unpinned (a live view following its head), so the
/// destination folds to the same point in the timeline.
function seqSearch(seq: number | null | undefined): StreamSearch {
  return seq != null ? { seq } : {};
}
