import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";

import {
  type TurnWindow,
  clampWindow,
  followWindow,
  initialWindow,
  pageInEnd,
  pageInStart,
  tailWindow,
  unseenTailCount,
} from "./transcriptWindowUtilities.ts";

/// How near the foot counts as "at the bottom": a small slack so a reader parked at the end still
/// follows even when the last line sits a few pixels short of the exact scroll limit.
const BOTTOM_SLACK_PX = 96;
/// How far outside the viewport a sentinel is allowed to page a chunk in — a generous margin so the
/// next chunk is loaded before the reader reaches the raw edge and never sees an empty gap.
const SENTINEL_MARGIN_PX = 800;

/// The two behaviours the transcript's scroll hook serves. `live` is the interactive conversation: it
/// opens on the tail (or a deep-linked turn), follows the foot while pinned, offers a jump-to-latest
/// pill when the reader has scrolled up, and windows the turns so the DOM stays bounded. `review` is a
/// read-only eval run: it keeps the whole transcript rendered and only follows the foot at the head —
/// the long-standing behaviour, unchanged, since an eval run is short and reviewed top to bottom.
export type TranscriptMode = "live" | "review";

/// The scroll surface Room drives the transcript through. `window` is the index range Transcript
/// renders (`null` in review mode → render everything); `topRef`/`bottomRef` are the page-in sentinels
/// Transcript mounts at the loaded range's edges; `pinned`, `newCount`, and `jumpToLatest` drive the
/// floating jump-to-latest pill.
export interface TranscriptScroll {
  window: TurnWindow | null;
  topRef: (node: HTMLElement | null) => void;
  bottomRef: (node: HTMLElement | null) => void;
  showJump: boolean;
  newCount: number;
  jumpToLatest: () => void;
}

/// Own the transcript's scroll: the render window, the follow-the-foot pinning, and the jump-to-latest
/// indicator. Room remounts this hook per room (its `<Room>` is keyed by the channel), so switching
/// rooms resets the window to the new conversation's tail, pinned. `active` is the head gate (a scrub
/// back into history leaves the reader undisturbed); `total` is the folded turn count; `focusIndex` is
/// a deep-linked turn's position (the window opens around it, not the tail); `footSignal` changes
/// whenever the transcript's foot moves (a turn lands, a token streams in, the optimistic echo or the
/// thinking pulse toggles); `inflightActive` is whether a generation is streaming at the tail (it
/// counts as new activity for the indicator when the reader is not pinned).
export function useTranscriptScroll({
  mode,
  active,
  total,
  focusIndex,
  footSignal,
  inflightActive,
}: {
  mode: TranscriptMode;
  active: boolean;
  total: number;
  focusIndex: number | null;
  footSignal: string;
  inflightActive: boolean;
}): TranscriptScroll {
  // The window opens on the tail, or centred on a deep-linked turn. A deep link lands unpinned so the
  // arrival wash (TurnItem's own scrollIntoView) wins over a tail jump; a fresh tail lands pinned.
  const [win, setWin] = useState<TurnWindow>(() => initialWindow(total, focusIndex));
  const [pinned, setPinned] = useState<boolean>(mode === "live" && active && focusIndex === null);

  // The rendered window. While a live reader is pinned at the head, it is *derived* as the tail rather
  // than stored — so new turns snap into view with no state write per turn (which would cascade
  // renders). Otherwise it is the stored, paged window (a deep link, or a reader scrolled up into
  // history), clamped in case a cursor scrub shrank the turn count under it.
  const view = mode === "live" && active && pinned ? followWindow(total) : clampWindow(win, total);
  // Refs mirror the render values so the scroll listener and the intersection observers — which fire
  // outside React's render — read the current window, count, and pinned state without stale closures.
  // Synced in the layout effect just below (writing them during render is disallowed).
  const winRef = useRef(view);
  const totalRef = useRef(total);
  const pinnedRef = useRef(pinned);
  const activeRef = useRef(active);
  useLayoutEffect(() => {
    winRef.current = view;
    totalRef.current = total;
    pinnedRef.current = pinned;
    activeRef.current = active;
  });

  // Whether the reader sat at the foot as of the last scroll or resize — read at the instant new
  // content lands, when the freshly grown page would itself read as "not at the bottom".
  const atBottomRef = useRef(pinned);
  // A pending top-prepend's pre-growth metrics, so the layout effect can restore the scroll position
  // after the older chunk inserts (measure `scrollHeight` before, adjust `scrollTop` by the delta).
  const prependRef = useRef<{ height: number; scrollY: number } | null>(null);
  // The review path skips the very first foot pass, recording the baseline rather than jumping.
  const primed = useRef(false);

  const scrollToFoot = () => window.scrollTo({ top: document.documentElement.scrollHeight });

  // Track whether the reader is at the foot, and in live mode reflect it into `pinned` so the pill and
  // the follow both react. Only transitions flip the state, so scrolling does not spam re-renders.
  useEffect(() => {
    const check = () => {
      const doc = document.documentElement;
      const at = doc.scrollHeight - window.scrollY - window.innerHeight <= BOTTOM_SLACK_PX;
      atBottomRef.current = at;
      if (mode === "live" && activeRef.current && at !== pinnedRef.current) {
        // Leaving the foot: anchor the stored base at the tail the reader is scrolling away from, so
        // paging up starts from there (while pinned the window was only derived, never stored). On a
        // fast fling the top sentinel's page-in can land in the same batch; its write (flagged by the
        // pending prepend) is the fresher truth and must not be clobbered by the anchor.
        if (!at) setWin((w) => (prependRef.current ? w : followWindow(totalRef.current)));
        setPinned(at);
      }
    };
    check();
    window.addEventListener("scroll", check, { passive: true });
    window.addEventListener("resize", check, { passive: true });
    return () => {
      window.removeEventListener("scroll", check);
      window.removeEventListener("resize", check);
    };
  }, [mode]);

  // While a live reader is pinned, any late height growth re-pins the foot. Content can grow without
  // a scroll event or a foot-signal change — outcome rows and briefs folding in after the initial
  // render, fonts and images settling — leaving the first paint parked a little short of the true
  // end. A prepend in flight defers to its own restore effect below; an unpinned reader is never
  // yanked.
  useEffect(() => {
    if (mode !== "live") return;
    const observer = new ResizeObserver(() => {
      if (prependRef.current) return;
      if (activeRef.current && pinnedRef.current) scrollToFoot();
    });
    observer.observe(document.body);
    return () => observer.disconnect();
  }, [mode]);

  // Restore the scroll position after a top-prepend grows the page, so the content the reader was
  // looking at stays put rather than jumping down by the inserted chunk's height. Deliberately
  // unkeyed: it must run on whichever render commits after a prepend flagged itself — keying on
  // `view.start` could strand the flag (wedging the observers and the resize re-pin behind it) if a
  // racing write left the start unchanged. A no-op unless a prepend is pending.
  useLayoutEffect(() => {
    const pending = prependRef.current;
    if (!pending) return;
    prependRef.current = null;
    const delta = document.documentElement.scrollHeight - pending.height;
    if (delta !== 0) window.scrollTo({ top: pending.scrollY + delta });
  });

  // Follow the foot. Live: while pinned and at the head, keep the foot in view as it moves. Review:
  // the long-standing behaviour — skip the first prime, then follow only when the reader is at the
  // foot, and never while scrubbed off the head. A prepend in flight defers to the effect above.
  useLayoutEffect(() => {
    if (prependRef.current) return;
    if (mode === "review") {
      if (!active) {
        primed.current = false;
        return;
      }
      if (!primed.current) {
        primed.current = true;
        return;
      }
      if (atBottomRef.current) scrollToFoot();
      return;
    }
    if (active && pinned) scrollToFoot();
  }, [mode, active, pinned, footSignal, total, view.end]);

  // Page in the previous chunk when the top sentinel nears the viewport, preserving scroll position by
  // recording the pre-growth metrics for the layout effect above.
  const [topNode, setTopNode] = useState<HTMLElement | null>(null);
  useEffect(() => {
    if (!topNode) return;
    const observer = new IntersectionObserver(
      (entries) => {
        if (!entries[0]?.isIntersecting) return;
        const current = winRef.current;
        if (current.start <= 0) return;
        prependRef.current = {
          height: document.documentElement.scrollHeight,
          scrollY: window.scrollY,
        };
        setWin(pageInStart(current));
      },
      { rootMargin: `${SENTINEL_MARGIN_PX}px 0px 0px 0px` },
    );
    observer.observe(topNode);
    return () => observer.disconnect();
    // Recreated whenever the loaded head moves: an observer only fires on *transitions*, so if a
    // prepended chunk renders shorter than the margin the sentinel never leaves it and true → true
    // would stall paging for good (observed at an idle gap, where dividers and terse turns make for
    // short chunks). A fresh observe() reports current state, so a sentinel still inside the margin
    // immediately pages the next chunk, cascading until it is genuinely clear or history runs out.
  }, [topNode, view.start]);

  // Page in the next chunk when the bottom sentinel nears the viewport — the reader scrolling back
  // down toward the tail (from a deep link, or after reading up into history). A pinned reader already
  // has the tail, so it never fires for them.
  const [bottomNode, setBottomNode] = useState<HTMLElement | null>(null);
  useEffect(() => {
    if (!bottomNode) return;
    const observer = new IntersectionObserver(
      (entries) => {
        if (!entries[0]?.isIntersecting) return;
        if (pinnedRef.current) return;
        const current = winRef.current;
        if (current.end >= totalRef.current) return;
        setWin(pageInEnd(current, totalRef.current));
      },
      { rootMargin: `0px 0px ${SENTINEL_MARGIN_PX}px 0px` },
    );
    observer.observe(bottomNode);
    return () => observer.disconnect();
    // Recreated on the loaded foot moving, for the same stalled-transition reason as the top observer.
  }, [bottomNode, view.end]);

  const jumpToLatest = useCallback(() => {
    setWin(tailWindow(totalRef.current));
    setPinned(true);
    atBottomRef.current = true;
    // The window may already be the tail (only the pill's presence lagged), so the follow effects
    // would not re-fire; scroll on the next frame regardless, once the tail turns have mounted.
    requestAnimationFrame(scrollToFoot);
  }, []);

  const newCount = mode === "live" ? unseenTailCount(view, total) : 0;
  const showJump = mode === "live" && active && !pinned && (newCount > 0 || inflightActive);

  return {
    window: mode === "live" ? view : null,
    topRef: setTopNode,
    bottomRef: setBottomNode,
    showJump,
    newCount,
    jumpToLatest,
  };
}
