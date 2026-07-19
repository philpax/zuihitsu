import { useContext, useState } from "react";

import { RefChip, RefChipAnchor, UnresolvedRefChip } from "../../components/RefChip.tsx";
import { type RefPopupPlacement, useRefPopup } from "../../components/refChipUtilities.ts";
import { useOptionalStream } from "../../lib/nav/useStreamLocation.ts";
import { MEM_CHIP_HANDLE_SIGIL, type MemPreview, MemRefs } from "../../lib/view/memRefs.ts";

// Memory references in rendered transcript text: a scanned memory-reference token (lifted by the combined scan)
// or a console State-view deep link (matched by the anchor override) renders as an inline chip labeled
// with the memory's handle and linking to its State view — the memory counterpart to `TurnRefChip`.
// Hovering previews the memory (its description and most recent entries) from the folded replica, in
// the same visual register as the turn preview (the console is the operator's surface, so no audience
// filtering here). The resolver context lives in `lib/view/memRefs.ts`; the shared chip presentation
// in `components/RefChip.tsx`.

/// An inline memory-reference chip: faint mono, labeled with the memory's handle, linking to the State
/// view at the timeline cursor. An id payload (`mem-chip:<id>`, from a token) resolves through the
/// graph to its `same_as` class primary; a handle payload (`mem-chip:@<handle>`, from a matched
/// State-view URL) resolves the handle directly. Hover (or focus) opens the preview popup. A reference
/// the graph does not resolve — unknown, or past the cursor — renders as a muted non-link chip, in the
/// same quiet register as an unknown turn.
export function MemRefChip({ payload }: { payload: string }) {
  const resolver = useContext(MemRefs);
  const stream = useOptionalStream();
  const { anchor, placement, show, hide } = useRefPopup();
  const [preview, setPreview] = useState<MemPreview | null>(null);
  const isHandle = payload.startsWith(MEM_CHIP_HANDLE_SIGIL);
  const target = isHandle
    ? resolver.byHandle(payload.slice(MEM_CHIP_HANDLE_SIGIL.length))
    : resolver.byId(payload);

  // Unresolved, or nowhere to link (a tree rendered outside a stream frame): degrade to a quiet chip
  // labeled with the handle if one is known, else the raw token.
  if (target === null || stream === null) {
    const label = isHandle ? payload.slice(MEM_CHIP_HANDLE_SIGIL.length) : payload;
    return (
      <UnresolvedRefChip
        title={`memory ${label} — not in view: an unknown handle, or a memory past the timeline cursor`}
      >
        {label}
      </UnresolvedRefChip>
    );
  }

  const { seq, link } = stream;
  // The preview's detail read is heavyweight, so it is fetched when the popup opens rather than every
  // render — resolving the target's handle to its description and recent entries at the fold cursor.
  const openPreview = () => {
    show();
    setPreview(resolver.preview(target.handle));
  };
  const closePreview = () => {
    hide();
    setPreview(null);
  };
  return (
    <RefChipAnchor anchor={anchor} onShow={openPreview} onHide={closePreview}>
      <RefChip
        glyph="◆"
        to={link.state(target.handle, { seq })}
        title={`Open ${target.handle} in State`}
      >
        {target.handle}
      </RefChip>
      {placement && preview && (
        <MemRefPopup handle={target.handle} preview={preview} placement={placement} />
      )}
    </RefChipAnchor>
  );
}

/// The hover preview: the memory's handle as a header, its description, and its most recent few
/// content entries as compact clamped lines. Spans throughout (the chip can sit inside a `<p>`),
/// styled as a raised card in the house tokens — the memory counterpart to `TurnRefPopup`.
function MemRefPopup({
  handle,
  preview,
  placement,
}: {
  handle: string;
  preview: MemPreview;
  placement: RefPopupPlacement;
}) {
  const empty = preview.description === "" && preview.entries.length === 0;
  return (
    <span
      className={
        "absolute z-20 block w-80 max-w-[75vw] rounded-sm border border-line bg-paper-raised p-3 shadow-lg " +
        (placement.below ? "top-full mt-1.5 " : "bottom-full mb-1.5 ") +
        (placement.alignRight ? "right-0" : "left-0")
      }
    >
      <span className="block font-mono text-2xs tracking-widest text-clay uppercase">{handle}</span>
      {preview.description !== "" && (
        <span className="mt-0.5 line-clamp-2 block text-xs/relaxed text-ink-soft">
          {preview.description}
        </span>
      )}
      {preview.entries.map((entry) => (
        <span key={entry.id} className="mt-2 line-clamp-2 block text-xs/relaxed text-ink">
          {entry.text}
        </span>
      ))}
      {empty && (
        <span className="mt-0.5 block text-xs/relaxed text-ink-faint">no visible content</span>
      )}
    </span>
  );
}
