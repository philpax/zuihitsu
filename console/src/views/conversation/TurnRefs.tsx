import { useContext } from "react";

import { RefChip, RefChipAnchor, UnresolvedRefChip } from "../../components/RefChip.tsx";
import { useRefPopup } from "../../components/refChipUtilities.ts";
import { formatDateTime, formatTime } from "../../lib/format/format.ts";
import { useStream } from "../../lib/nav/useStreamLocation.ts";
import { type TurnRefTarget, TurnRefs, speakerLabel } from "../../lib/view/turnRefs.ts";

// Turn references in rendered transcript text (spec §Conversations → Transcript references). Turn
// text is scanned through the wasm `turn_ref` parser — the same definition the agent's resolver
// reads — and each turn-reference token or pasted deep-link URL renders as an inline chip labeled
// with the referenced turn's speaker. Both sides' text runs through the Markdown pipeline, whose
// remark plugin mints the `turn-chip:` links this chip renders. Hovering previews the moment and a
// couple of neighbors either side from the folded replica (the console is the operator's surface, so
// no audience filtering here); clicking follows the deep link the transcript already honors (scroll +
// arrival wash). The lookup context and the remark plugin live in `lib/turnRefs.ts`.

/// An inline reference chip: faint mono, labeled with the referenced turn's speaker. Hover (or
/// focus) opens the preview popup; click follows the deep link. An id the fold does not hold —
/// unknown, or past the timeline cursor — renders in the quiet-notice register instead.
export function TurnRefChip({ id }: { id: string }) {
  const targets = useContext(TurnRefs);
  const { seq, link } = useStream();
  const { anchor, placement, show, hide } = useRefPopup();
  const target = targets.get(id);
  if (!target) {
    return (
      <UnresolvedRefChip
        title={`turn ${id} — not in view: an unknown id, or a moment past the timeline cursor`}
      >
        unknown turn
      </UnresolvedRefChip>
    );
  }
  // The deep link opens the Conversation view on the room that holds the turn
  // (`…/conversation/<room>`) with the turn pinned in `?turn` — the same URL shape the transcript's
  // timestamp anchors mint, reached identically whether the chip renders inside the Conversation view
  // or outside it (Events, Background). The timeline cursor rides along when pinned.
  return (
    <RefChipAnchor anchor={anchor} onShow={show} onHide={hide}>
      <RefChip
        glyph="↩"
        to={link.conversation({ room: target.roomKey, turn: target.turn.turnId, seq })}
      >
        <span className="text-ink-faint">{target.roomKey}</span>
        <span aria-hidden className="text-ink-faint">
          ·
        </span>
        {speakerLabel(target.turn)}
      </RefChip>
      {placement && <TurnRefPopup target={target} placement={placement} />}
    </RefChipAnchor>
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
                  "font-mono text-2xs tracking-widest uppercase " +
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
                "mt-0.5 line-clamp-2 block text-xs/relaxed " +
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
