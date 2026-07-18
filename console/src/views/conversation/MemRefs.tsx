import { useContext } from "react";

import { Link } from "../../lib/nav/history.tsx";
import { useOptionalStream } from "../../lib/nav/useStreamLocation.ts";
import { MEM_CHIP_HANDLE_SIGIL, MemRefs } from "../../lib/view/memRefs.ts";

// Memory references in rendered transcript text: a scanned memory-reference token (lifted by the combined scan)
// or a console State-view deep link (matched by the anchor override) renders as an inline chip labeled
// with the memory's handle and linking to its State view — the memory counterpart to `TurnRefChip`.
// The resolver context lives in `lib/view/memRefs.ts`.

/// An inline memory-reference chip: faint mono, labeled with the memory's handle, linking to the State
/// view at the timeline cursor. An id payload (`mem-chip:<id>`, from a token) resolves through the
/// graph to its `same_as` class primary; a handle payload (`mem-chip:@<handle>`, from a matched
/// State-view URL) resolves the handle directly. A reference the graph does not resolve — unknown, or
/// past the cursor — renders as a muted non-link chip, in the same quiet register as an unknown turn.
export function MemRefChip({ payload }: { payload: string }) {
  const resolver = useContext(MemRefs);
  const stream = useOptionalStream();
  const isHandle = payload.startsWith(MEM_CHIP_HANDLE_SIGIL);
  const target = isHandle
    ? resolver.byHandle(payload.slice(MEM_CHIP_HANDLE_SIGIL.length))
    : resolver.byId(payload);

  // Unresolved, or nowhere to link (a tree rendered outside a stream frame): degrade to a quiet chip
  // labeled with the handle if one is known, else the raw token.
  if (target === null || stream === null) {
    const label = isHandle ? payload.slice(MEM_CHIP_HANDLE_SIGIL.length) : payload;
    return (
      <span
        title={`memory ${label} — not in view: an unknown handle, or a memory past the timeline cursor`}
        className="mx-0.5 inline-flex items-baseline rounded-sm border border-dashed border-line px-1.5 font-mono text-2xs text-ink-faint"
      >
        {label}
      </span>
    );
  }

  const { seq, link } = stream;
  return (
    <Link
      to={link.state(target.handle, { seq })}
      title={`Open ${target.handle} in State`}
      className="mx-0.5 inline-flex items-baseline gap-1 rounded-sm border border-line bg-oat/40 px-1.5 font-mono text-2xs text-ink-soft no-underline transition-colors hover:border-line-strong hover:text-ink"
    >
      <span aria-hidden className="text-ink-faint">
        ◆
      </span>
      {target.handle}
    </Link>
  );
}
