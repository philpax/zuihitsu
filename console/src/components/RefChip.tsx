import type { ReactNode, RefObject } from "react";

import { Link } from "../lib/nav/history.tsx";
import type { AppLocation } from "../lib/nav/location.ts";

// The shared presentation for the inline reference chips — turn references
// (`views/conversation/TurnRefs.tsx`) and memory references (`views/conversation/MemRefs.tsx`) both
// render through this one layer, so a chip's look (a faint-mono resolved link, the muted dashed
// fallback) and its hover-preview placement are answered in a single place. Presentational only: how
// a reference resolves, and what its preview shows, stay with each chip.

/// A resolved reference chip: a faint-mono link into the referenced subject, led by a faint glyph.
/// The caller supplies the destination, the glyph, and the label (compound for a turn's room·speaker,
/// a bare handle for a memory). The hover preview, when there is one, is a sibling of this link under
/// [`RefChipAnchor`], not part of the chip itself.
export function RefChip({
  to,
  title,
  glyph,
  children,
}: {
  to: AppLocation;
  title?: string;
  glyph: string;
  children: ReactNode;
}) {
  return (
    <Link
      to={to}
      title={title}
      className="inline-flex items-baseline gap-1 rounded-sm border border-line bg-oat/40 px-1.5 font-mono text-2xs text-ink-soft no-underline transition-colors hover:border-line-strong hover:text-ink"
    >
      <span aria-hidden className="text-ink-faint">
        {glyph}
      </span>
      {children}
    </Link>
  );
}

/// The muted fallback for a reference the fold does not resolve — an unknown subject, or one past the
/// timeline cursor: a dashed non-link chip in the quiet-notice register, labeled and carrying a
/// `title` that explains the absence. Shared verbatim by both chip kinds.
export function UnresolvedRefChip({ title, children }: { title: string; children: ReactNode }) {
  return (
    <span
      title={title}
      className="mx-0.5 inline-flex items-baseline rounded-sm border border-dashed border-line px-1.5 font-mono text-2xs text-ink-faint"
    >
      {children}
    </span>
  );
}

/// The positioning wrapper a previewable chip sits in: a relative inline span that the preview
/// anchors against, driving the preview's open and close on hover and focus. The preview popup itself
/// is the caller's — passed as a child beside the chip — so each reference kind renders its own
/// contents in the shared visual register.
export function RefChipAnchor({
  anchor,
  onShow,
  onHide,
  children,
}: {
  anchor: RefObject<HTMLSpanElement | null>;
  onShow: () => void;
  onHide: () => void;
  children: ReactNode;
}) {
  return (
    <span
      ref={anchor}
      className="relative mx-0.5 inline-block"
      onMouseEnter={onShow}
      onMouseLeave={onHide}
      onFocus={onShow}
      onBlur={onHide}
    >
      {children}
    </span>
  );
}
