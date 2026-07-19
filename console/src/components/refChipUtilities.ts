import { type RefObject, useRef, useState } from "react";

// The non-component half of the shared reference-chip presentation (`components/RefChip.tsx`): the
// hover-preview placement measurement, shared by the turn and memory chips so it lives in one place.

/// The hover-preview placement, measured when the popup opens: it prefers above-left of the chip but
/// flips to the right edge or below when that would leave the viewport (a chip near the pane's right
/// edge or the window's top would otherwise clip its preview).
export interface RefPopupPlacement {
  alignRight: boolean;
  below: boolean;
}

/// Hover/focus state for a chip's preview: the anchor ref to measure against, the current placement
/// (`null` while closed), and the open/close callbacks. Placement is measured at open from the
/// anchor's rect, so the popup renders where it fits. Both chip kinds share this so the measurement
/// lives in one place.
export function useRefPopup(): {
  anchor: RefObject<HTMLSpanElement | null>;
  placement: RefPopupPlacement | null;
  show: () => void;
  hide: () => void;
} {
  const anchor = useRef<HTMLSpanElement>(null);
  const [placement, setPlacement] = useState<RefPopupPlacement | null>(null);
  const show = () => {
    const rect = anchor.current?.getBoundingClientRect();
    setPlacement({
      // w-80 is 20rem = 320px; 16px of breathing room against the edge.
      alignRight: rect !== undefined && rect.left + 336 > window.innerWidth,
      // A generous estimate of the popup's height; a five-line preview runs ~260px.
      below: rect !== undefined && rect.top < 280,
    });
  };
  const hide = () => setPlacement(null);
  return { anchor, placement, show, hide };
}
