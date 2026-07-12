import { useEffect, useLayoutEffect, useRef } from "react";

/// How near the foot counts as "at the bottom": a small slack so a reader parked at the end still
/// follows even when the last line sits a few pixels short of the exact scroll limit.
const BOTTOM_SLACK_PX = 96;

/// Keep the window pinned to the foot of the transcript while `active` and the reader is already at the
/// bottom, so new content at the tail — a landing turn, a streamed token, a committed deliberation step
/// — stays in view, without yanking a reader who has scrolled up into history. Serves both frames: the
/// live conversation and a read-only eval run watched at its head. `signal` must change whenever the
/// transcript's foot moves (a turn lands, a token streams in, the optimistic echo appears, the thinking
/// pulse toggles): the scroll fires only when it changes, and never on the first prime, so mounting or
/// returning to the head records the baseline rather than jumping.
export function useFollowBottom(active: boolean, signal: string) {
  // Whether the reader was at the foot as of the last scroll or resize — recorded continuously so it can
  // be read at the instant new content lands, when the freshly grown page would itself read as
  // "not at the bottom" and mislead a check made too late.
  const atBottom = useRef(true);
  const primed = useRef(false);

  useEffect(() => {
    if (!active) return;
    const check = () => {
      const doc = document.documentElement;
      atBottom.current = doc.scrollHeight - window.scrollY - window.innerHeight <= BOTTOM_SLACK_PX;
    };
    check();
    window.addEventListener("scroll", check, { passive: true });
    window.addEventListener("resize", check, { passive: true });
    return () => {
      window.removeEventListener("scroll", check);
      window.removeEventListener("resize", check);
    };
  }, [active]);

  useLayoutEffect(() => {
    if (!active) {
      primed.current = false;
      return;
    }
    // The first pass after activating only records the baseline: never scroll on mount or on a return
    // to the head, only on a later change to the foot.
    if (!primed.current) {
      primed.current = true;
      return;
    }
    if (atBottom.current) {
      window.scrollTo({ top: document.documentElement.scrollHeight });
    }
  }, [active, signal]);
}
