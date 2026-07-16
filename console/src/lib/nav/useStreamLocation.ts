import { useLocation, useNavigate, useParams, useSearch } from "@tanstack/react-router";

import { type ViewId, asViewId } from "./streamViews.ts";
import { viewPath } from "./routes.ts";

/// The active view and timeline cursor for a stream, read from and written to the URL: the `:view`
/// path segment and the `seq` search, both relative to `base` — a run's path under the eval frame, or
/// `/live` under the agent frame. Pulling this into one hook is what makes routing behave identically
/// in either frame: the same view tabs and the same scrubber move through the browser's history the
/// same way. `seq` is `null` at the head (following the latest state) or a pinned earlier seq. A
/// view's own navigational selection (the open memory, room, or settings tab) rides the trailing
/// `:selection` segment, read with [`useSelection`]; switching views drops it, since it is
/// view-specific.
export interface StreamLocation {
  view: ViewId | undefined;
  seq: number | null;
  selectView: (view: ViewId) => void;
  setSeq: (seq: number | null) => void;
}

/// The current stream's base path — the path with its trailing `:view` (and any `:selection`)
/// segment dropped, so a run's `/eval/:scenario/:run/:view/:selection?` yields `/eval/:scenario/:run`
/// and the agent's `/live/:view/:selection?` yields `/live`. Lets a view deep inside a stream (an
/// event's memory ref, a turn's room link) build a link to a sibling view without being told which
/// frame it lives in. Reads the route's own params rather than counting path segments, so an encoded
/// selection (a memory name, a room key) is dropped as a single segment.
export function useStreamBase(): string {
  const { pathname } = useLocation();
  const { selection } = useParams({ strict: false });
  // Drop the `:selection` segment first (present only on State, Conversation, and Settings), then the
  // `:view` segment, leaving the frame's base.
  const withoutSelection = selection !== undefined ? pathname.replace(/\/[^/]*$/, "") : pathname;
  return withoutSelection.replace(/\/[^/]*$/, "");
}

/// The active view, narrowed to a [`ViewId`] (or `undefined` on a bare or unknown segment).
export function useStreamView(): ViewId | undefined {
  return asViewId(useParams({ strict: false }).view);
}

/// The pinned timeline cursor, or `null` when a live view follows its head — the value the deep-link
/// builders carry along so a jump folds to the same point in the timeline.
export function useSeq(): number | null {
  return useSearch({ strict: false }).seq ?? null;
}

/// The active view's `:selection` segment — the open memory, room, or settings tab, decoded — or
/// `undefined` when the view carries none or sits at its default. Free-form (a memory name, a room
/// key), so `string`; a view with a closed set narrows it itself.
export function useSelection(): string | undefined {
  return useParams({ strict: false }).selection;
}

export function useStreamLocation(base: string): StreamLocation {
  const navigate = useNavigate();
  const view = useParams({ strict: false }).view;
  const seq = useSearch({ strict: false }).seq ?? null;

  return {
    view: asViewId(view),
    seq,
    // A tab move keeps only the cross-cutting `seq` cursor, dropping the `:selection` segment and the
    // leaving view's own search (`focus`, `turn`).
    selectView: (next) => navigate(viewPath(base, next, seq)),
    // Replace, not push, so dragging the scrubber does not bury the back button under a history entry
    // per step; the view path is left intact by staying on the current route (`to: "."`).
    setSeq: (next) =>
      navigate({
        to: ".",
        search: (prev) => ({ ...prev, seq: next ?? undefined }),
        replace: true,
      }),
  };
}
