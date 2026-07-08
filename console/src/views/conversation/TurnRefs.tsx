import { useContext, useRef, useState } from "react";
import { Link, useSearchParams } from "react-router-dom";

import { formatDateTime, formatTime } from "../../lib/format/format.ts";
import { scanTurnRefs } from "../../lib/replica/replica.ts";
import { type TurnRefTarget, TurnRefs, speakerLabel } from "../../lib/view/turnRefs.ts";

// Turn references in rendered transcript text (spec §Conversations → Transcript references). Turn
// text is scanned through the wasm `turn_ref` parser — the same definition the agent's resolver
// reads — and each `[turn:<ulid>]` token or pasted deep-link URL renders as an inline chip labeled
// with the referenced turn's speaker. Hovering previews the moment and a couple of neighbors either
// side from the folded replica (the console is the operator's surface, so no audience filtering
// here); clicking follows the deep link the transcript already honors (scroll + arrival wash). The
// lookup context and the Markdown-side remark plugin live in `lib/turnRefs.ts`.

/// Raw turn text with its turn references rendered as chips — the participant-turn counterpart of
/// the Markdown pipeline's remark plugin. Text without references passes through untouched.
export function RefText({ text }: { text: string }) {
  const segments = scanTurnRefs(text);
  if (!segments.some((segment) => segment.kind === "ref")) return <>{text}</>;
  return (
    <>
      {segments.map((segment, index) =>
        segment.kind === "prose" ? (
          <span key={index}>{segment.text}</span>
        ) : (
          <TurnRefChip key={index} id={segment.id} />
        ),
      )}
    </>
  );
}

/// An inline reference chip: faint mono, labeled with the referenced turn's speaker. Hover (or
/// focus) opens the preview popup; click follows the deep link. An id the fold does not hold —
/// unknown, or past the timeline cursor — renders in the quiet-notice register instead.
export function TurnRefChip({ id }: { id: string }) {
  const targets = useContext(TurnRefs);
  const [searchParams] = useSearchParams();
  const anchor = useRef<HTMLSpanElement>(null);
  // The popup's placement is measured at open: it prefers above-left of the chip, but flips to the
  // right edge or below when that would leave the viewport (a chip near the pane's right edge or the
  // window's top would otherwise clip its preview).
  const [open, setOpen] = useState<{ alignRight: boolean; below: boolean } | null>(null);
  const show = () => {
    const rect = anchor.current?.getBoundingClientRect();
    setOpen({
      // w-80 is 20rem = 320px; 16px of breathing room against the edge.
      alignRight: rect !== undefined && rect.left + 336 > window.innerWidth,
      // A generous estimate of the popup's height; a five-line preview runs ~260px.
      below: rect !== undefined && rect.top < 280,
    });
  };
  const target = targets.get(id);
  if (!target) {
    return (
      <span
        title={`turn ${id} — not in view: an unknown id, or a moment past the timeline cursor`}
        className="mx-0.5 inline-flex items-baseline rounded-sm border border-dashed border-line px-1.5 font-mono text-2xs text-ink-faint"
      >
        unknown turn
      </span>
    );
  }
  // The deep link keeps the current view's params (the timeline cursor rides along) and pins the
  // room and turn — the same URL shape the transcript's timestamp anchors mint.
  const params = new URLSearchParams(searchParams);
  params.set("room", target.roomKey);
  params.set("turn", target.turn.turnId);
  return (
    <span
      ref={anchor}
      className="relative mx-0.5 inline-block"
      onMouseEnter={show}
      onMouseLeave={() => setOpen(null)}
      onFocus={show}
      onBlur={() => setOpen(null)}
    >
      <Link
        to={{ search: params.toString() }}
        className="inline-flex items-baseline gap-1 rounded-sm border border-line bg-oat/40 px-1.5 font-mono text-2xs text-ink-soft no-underline transition-colors hover:border-line-strong hover:text-ink"
      >
        <span aria-hidden className="text-ink-faint">
          ↩
        </span>
        {speakerLabel(target.turn)}
      </Link>
      {open && <TurnRefPopup target={target} placement={open} />}
    </span>
  );
}

/// The hover preview: the focal turn and its neighbors as compact transcript lines — speaker,
/// timestamp, and clamped text. Spans throughout (the chip can sit inside a `<p>`), styled as a
/// raised card in the house tokens.
function TurnRefPopup({
  target,
  placement,
}: {
  target: TurnRefTarget;
  placement: { alignRight: boolean; below: boolean };
}) {
  return (
    <span
      className={
        "absolute z-20 block w-80 max-w-[75vw] rounded-sm border border-line bg-paper-raised p-3 shadow-lg " +
        (placement.below ? "top-full mt-1.5 " : "bottom-full mb-1.5 ") +
        (placement.alignRight ? "right-0" : "left-0")
      }
    >
      {target.window.map((turn, index) => {
        const focused = index === target.focusIndex;
        return (
          <span key={turn.turnId} className={"block" + (index > 0 ? " mt-2" : "")}>
            <span className="flex items-baseline gap-2">
              <span
                className={
                  "font-mono text-2xs uppercase tracking-widest " +
                  (focused ? (turn.role === "Agent" ? "text-sage" : "text-clay") : "text-ink-faint")
                }
              >
                {speakerLabel(turn)}
              </span>
              {turn.recordedAt > 0 && (
                <span
                  className="ml-auto shrink-0 font-mono text-2xs text-ink-faint"
                  title={formatDateTime(turn.recordedAt)}
                >
                  {formatTime(turn.recordedAt)}
                </span>
              )}
            </span>
            <span
              className={
                "mt-0.5 line-clamp-2 block text-xs leading-relaxed " +
                (focused ? "text-ink" : "text-ink-soft")
              }
            >
              {turn.text || (turn.role === "Agent" ? "(stayed silent)" : "(system)")}
            </span>
          </span>
        );
      })}
    </span>
  );
}
