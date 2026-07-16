import { useState, type ReactNode } from "react";
import { Link } from "react-router-dom";

import type { EventPayload } from "@zuihitsu/wire/types/EventPayload.ts";
import type { EventSource } from "@zuihitsu/wire/types/EventSource.ts";
import type { EventCategory } from "../lib/model/events.ts";
import { CATEGORY_COLOR } from "../lib/model/events.ts";
import { useStreamBase } from "../lib/nav/useStreamLocation.ts";
import { EventDetail } from "./EventDetail.tsx";

/// The shared shape of an expandable event row — the fields both [`TurnOutcome`] and
/// [`BackgroundEvent`] carry. Extracting it keeps the Conversation view's turn outcomes and the
/// Background view's rows in sync as the [`EventDetail`] rendering evolves.
export interface EventRowData {
  seq: number;
  recordedAt: number;
  /// The envelope's authoring authority, shown as faint provenance in the expanded detail.
  source: EventSource;
  type: EventPayload["type"];
  category: EventCategory;
  summary: string;
  payload: EventPayload;
}

/// A one-line event summary that expands, in place, into the same specialized viewer the Events
/// tab uses. Shared by the Conversation view's turn outcomes and the Background view's rows. When
/// `triggeredBy` is present, a dim annotation renders below the summary line linking back to the
/// conversation turn that triggered the background pass.
export function EventRow({
  row,
  nameById,
  conversationNameById,
  triggeredBy,
}: {
  row: EventRowData;
  nameById: Map<string, string>;
  conversationNameById: Map<string, string>;
  triggeredBy?: {
    speaker: string | null;
    text: string;
    platform: string;
    scopePath: string;
  } | null;
}) {
  const [open, setOpen] = useState(false);
  const base = useStreamBase();
  return (
    <li className="font-mono text-xs">
      <button
        onClick={() => setOpen(!open)}
        className="group flex w-full items-baseline gap-2 text-left"
        title={open ? "Collapse" : "Expand the event"}
      >
        <span className="text-ink-faint">↳</span>
        <span className={CATEGORY_COLOR[row.category]}>{row.type}</span>
        <span
          className={
            "truncate transition-colors " +
            (open ? "text-ink" : "text-ink-soft group-hover:text-ink")
          }
        >
          {row.summary}
        </span>
      </button>
      {triggeredBy && <TriggeredBy {...triggeredBy} base={base} />}
      {open && (
        <div className="my-1 ml-4 border-l-2 border-line py-1 pl-3">
          <EventDetail
            payload={row.payload}
            nameById={nameById}
            conversationNameById={conversationNameById}
            base={base}
            seq={row.seq}
            recordedAt={row.recordedAt}
            source={row.source}
          />
        </div>
      )}
    </li>
  );
}

/// A dim, clickable annotation below a background-pass row linking back to the conversation turn
/// that last touched its memory before the pass ran. The annotation shows the triggering turn's
/// speaker and a truncated snippet of its text. Clicking navigates to the Conversation view with
/// the triggering room selected (turn-level focus is future work).
function TriggeredBy({
  speaker,
  text,
  platform,
  scopePath,
  base,
}: {
  speaker: string | null;
  text: string;
  platform: string;
  scopePath: string;
  base: string;
}): ReactNode {
  const room = `${platform} · ${scopePath}`;
  const snippet = text.replace(/\s+/g, " ").trim();
  const label = speaker ? `after ${speaker}'s turn` : "after the agent's turn";
  const to = `${base}/conversation?room=${encodeURIComponent(room)}`;
  return (
    <div className="mt-0.5 ml-4">
      <Link
        to={to}
        className="text-ink-faint transition-colors hover:text-clay"
        title={`Open the conversation in ${room}`}
      >
        {label}
        {" · "}
        <span className="italic">
          {snippet.length > 60 ? `“${snippet.slice(0, 60)}…”` : `“${snippet}”`}
        </span>
      </Link>
    </div>
  );
}
