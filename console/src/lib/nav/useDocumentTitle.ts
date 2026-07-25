import { useEffect } from "react";

/// Set the browser tab title from the active context and, where one is active, the sub-view,
/// following the `·`-separated convention the header eyebrows already use. Several tabs open against
/// the same console — a running agent, the eval viewer, the landing — are then distinguishable in the
/// tab strip and the window switcher without clicking through them. The static
/// `<title>zuihitsu</title>` in `index.html` is the pre-hydration fallback; once a frame mounts it
/// takes over `document.title`, and on unmount the title is restored so navigating away does not
/// leave a stale context in the strip.
///
/// The view is the lowercase id from the URL (`conversation`, `state`, …), the same value
/// [`useStreamLocation`] and the eval frame's `useMatch` read — so the title derives from the same
/// URL the views already read. Omit it (or pass `null`/`undefined`) for a context with no active
/// sub-view, like the landing or the eval overview. A `null` context drops the segment entirely —
/// the live agent is the console's default face, so its tabs read `zuihitsu · <view>` with no
/// context label.
export function useDocumentTitle(context: string | null, view?: string | null) {
  useEffect(() => {
    const previous = document.title;
    document.title = ["zuihitsu", context, view].filter(Boolean).join(" · ");
    return () => {
      document.title = previous;
    };
  }, [context, view]);
}
