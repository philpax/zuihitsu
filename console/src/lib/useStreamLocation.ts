import { useLocation, useNavigate, useParams, useSearchParams } from "react-router-dom";

/// The active view and timeline cursor for a stream, read from and written to the URL: the `:view`
/// path segment and the `?seq` query, both relative to `base` — a run's path under the eval frame, or
/// `/live` under the agent frame. Pulling this into one hook is what makes routing behave identically
/// in either frame: the same view tabs and the same scrubber move through the browser's history the
/// same way. `seq` is `null` at the head (following the latest state) or a pinned earlier seq.
export interface StreamLocation {
  view: string | undefined;
  seq: number | null;
  selectView: (view: string) => void;
  setSeq: (seq: number | null) => void;
}

/// The current stream's base path — the path with its trailing `:view` segment dropped, so a run's
/// `/eval/:scenario/:run/:view` yields `/eval/:scenario/:run` and the agent's `/live/:view` yields
/// `/live`. Lets a view deep inside a stream (an event's memory ref) build a link to a sibling view
/// without being told which frame it lives in.
export function useStreamBase(): string {
  return useLocation().pathname.replace(/\/[^/]*$/, "");
}

export function useStreamLocation(base: string): StreamLocation {
  const navigate = useNavigate();
  const params = useParams();
  const [searchParams, setSearchParams] = useSearchParams();

  // `seq=0` is a valid cursor (before the first event), so test for presence, not truthiness.
  const raw = searchParams.get("seq");
  const seq = raw !== null && raw !== "" ? Number(raw) : null;

  return {
    view: params.view,
    seq,
    selectView: (view) =>
      navigate({ pathname: `${base}/${view}`, search: searchParams.toString() }),
    // Replace, not push, so dragging the scrubber does not bury the back button under a history entry
    // per step; the view path is left intact.
    setSeq: (next) =>
      setSearchParams(
        (prev) => {
          const updated = new URLSearchParams(prev);
          if (next === null) updated.delete("seq");
          else updated.set("seq", String(next));
          return updated;
        },
        { replace: true },
      ),
  };
}
