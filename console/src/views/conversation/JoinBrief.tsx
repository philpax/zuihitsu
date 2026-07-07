import { type ComponentProps, type RefObject, useState } from "react";
import { useNavigate } from "react-router-dom";
import { motion } from "motion/react";

import type { Brief } from "../../types/Brief.ts";
import type { TurnModel } from "../../lib/model/conversation.ts";
import { useStreamBase } from "../../lib/nav/useStreamLocation.ts";
import { statePath } from "../../lib/nav/routes.ts";
import { linkedClass } from "./turnUtilities.ts";
import { TurnTimeAnchor } from "./Turn.tsx";

/// A mid-session join, drawn as an entrance seam: a labelled rule whose centered label is a disclosure
/// into the pretty-printed brief the joiner arrived with. Collapsed by default — the seam reads at a
/// glance ("priya entered · join brief"), and the brief opens on demand. This replaces surfacing the
/// raw brief markup the turn's `text` carries, which read as leakage of the composer's internal format.
export function JoinBriefTurn({
  turn,
  roomKey,
  linked,
  enter,
  itemRef,
}: {
  turn: TurnModel;
  roomKey: string;
  linked: boolean;
  enter: ComponentProps<typeof motion.li>;
  itemRef: RefObject<HTMLLIElement | null>;
}) {
  const [open, setOpen] = useState(false);
  const who = turn.speaker ?? turn.brief?.subject ?? "someone";
  return (
    <motion.li ref={itemRef} className={"py-3" + linkedClass(linked)} {...enter}>
      <div className="flex items-center gap-3">
        <span className="h-px flex-1 bg-line" />
        <button
          onClick={() => setOpen(!open)}
          className="flex items-baseline gap-2 font-mono text-2xs text-ink-faint transition-colors hover:text-ink"
        >
          <span aria-hidden className="inline-block w-3 shrink-0 text-center">
            {open ? "▾" : "▸"}
          </span>
          <span>{who} entered</span>
          <span className="text-ink-faint/70">· join brief</span>
        </button>
        {turn.recordedAt > 0 && (
          <TurnTimeAnchor roomKey={roomKey} turnId={turn.turnId} recordedAt={turn.recordedAt} />
        )}
        <span className="h-px flex-1 bg-line" />
      </div>
      {open && turn.brief && (
        <div className="mx-auto mt-3 max-w-prose">
          <JoinBriefBody brief={turn.brief} seq={turn.seq} />
        </div>
      )}
    </motion.li>
  );
}

/// The pretty-printed join brief, read straight from its structured parts (nothing is parsed back out
/// of the markup): the summary as prose, the recent facts as a compact list with each fact's
/// provenance/staleness markers set quietly beside it, and the relationships as `relation → name` with
/// each name opening the memory in the State view at this moment in the timeline.
export function JoinBriefBody({ brief, seq }: { brief: Brief; seq: number }) {
  const base = useStreamBase();
  const navigate = useNavigate();
  return (
    <div className="space-y-3 border-l-2 border-line pl-4 text-sm">
      {brief.summary && <p className="leading-relaxed text-ink-soft">{brief.summary}</p>}
      {brief.recent_facts.length > 0 && (
        <ul className="space-y-1">
          {brief.recent_facts.map((fact, index) => (
            <li key={index} className="flex flex-wrap items-baseline gap-x-2 text-ink">
              <span>{fact.text}</span>
              {fact.markers.map((marker, markerIndex) => (
                <span key={markerIndex} className="font-mono text-2xs text-ink-faint">
                  {marker}
                </span>
              ))}
            </li>
          ))}
        </ul>
      )}
      {brief.relationships.length > 0 && (
        <ul className="space-y-1 font-mono text-xs text-ink-soft">
          {brief.relationships.map((relationship, index) => (
            <li key={index} className="flex items-baseline gap-2">
              <span className="text-ink-faint">{relationship.relation}</span>
              <span aria-hidden className="text-ink-faint">
                →
              </span>
              <button
                onClick={() => navigate(statePath(base, seq, relationship.subject))}
                title={`Open ${relationship.subject} in State`}
                className="text-clay underline-offset-2 transition-colors hover:text-ink hover:underline"
              >
                {relationship.subject}
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
