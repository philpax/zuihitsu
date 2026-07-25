import { useState } from "react";

import type { Event } from "@zuihitsu/wire/types/Event.ts";
import type { EventPayload } from "@zuihitsu/wire/types/EventPayload.ts";
import type { Replica } from "../lib/replica/replica.ts";
import { nameById } from "../lib/model/labels.ts";
import { buildBackgroundEvents, type BackgroundEvent } from "../lib/model/conversation.ts";
import { conversationNameById } from "../lib/model/conversationNameById.ts";
import { Link } from "../lib/nav/history.tsx";
import { useNavigate } from "../lib/nav/historyContext.ts";
import { useOptionalStream, useStream } from "../lib/nav/useStreamLocation.ts";
import { Eyebrow, Segmented } from "../components/primitives.tsx";
import { EventDetail } from "../components/EventDetail.tsx";
import { LogEventRow } from "../components/LogEventRow.tsx";
import { Pager } from "../components/Pager.tsx";
import { pageOf } from "../components/pagerUtilities.ts";

/// The Background view: the background passes' (describer, temporal extraction, belief
/// arbitration, link-inference) log-only audit events, collected from the run's event stream and
/// grouped by pass type. These
/// events carry no conversation or turn attribution — they run asynchronously, potentially long
/// after the turn that inspired them — so they surface here as a top-level timeline alongside the
/// Conversation view rather than mis-attributed to a turn or silently dropped. Each pass type is a
/// subtab; within it the events read newest first, paged at [`PAGE_SIZE`], each row expanding in
/// place into the same specialized viewer the Events tab uses, with a "triggered by" annotation in
/// the detail linking back to the conversation turn that last touched the memory before the pass ran.
///
/// The open pass category rides in the URL as the view's selection segment (`/…/background/<category>`),
/// so a subtab is a shareable deep link and browser back and forward walk the subtab history — the same
/// register the Settings view's section uses. A category absent from the log at the current cursor (a
/// scrub emptied it) falls back to the first present group, so a stale segment never strands the view.
/// The page and the expanded row stay component-local: they are within-view convenience that resets on
/// each subtab change, not navigational state.
export function BackgroundView({
  replica,
  events,
  cursor,
}: {
  replica: Replica;
  events: Event[];
  cursor: number;
}) {
  const names = nameById(replica.memories(""));
  const convNames = conversationNameById(replica.conversations());
  const background = buildBackgroundEvents(events, names, cursor);
  const groups = groupByPass(background);

  const navigate = useNavigate();
  const { selection, seq, link } = useStream();
  const [page, setPage] = useState(0);
  const [expanded, setExpanded] = useState<number | null>(null);

  // The active subtab is the URL selection segment; it defaults to the first non-empty group, and if
  // the segment no longer names a present group (a cursor scrub emptied it), it falls back to the first
  // present one — so a stale deep link degrades rather than showing an empty pane.
  const active =
    selection && groups.some((group) => group.id === selection)
      ? selection
      : (groups[0]?.id ?? null);
  const activeGroup = groups.find((group) => group.id === active) ?? null;
  const rows = activeGroup ? activeGroup.events : [];
  const paged = pageOf(rows, page);

  // Switching subtab is navigation (a pushed history entry), so back and forward step between
  // categories. The page and the expanded row are component-local convenience, reset here so the new
  // category opens at its first page with nothing expanded.
  function selectGroup(id: string) {
    setPage(0);
    setExpanded(null);
    navigate(link.view("background", { selection: id, seq }));
  }

  return (
    <section>
      <div className="mb-3 flex items-baseline justify-between gap-4">
        <Eyebrow>{background.length} background events</Eyebrow>
        <Eyebrow>
          seq 1 – {cursor}
          {cursor < events.length ? ` of ${events.length}` : ""}
        </Eyebrow>
      </div>

      {groups.length === 0 || active === null ? (
        <p className="font-mono text-2xs text-ink-faint">no background passes recorded</p>
      ) : (
        <>
          <Segmented
            options={groups.map((group) => ({
              id: group.id,
              label: `${group.label} (${group.events.length})`,
            }))}
            value={active}
            onChange={selectGroup}
            className="mb-6"
          />
          <div className="border-t border-line/60">
            {paged.map((event) => (
              <LogEventRow
                key={event.seq}
                type={event.type}
                category={event.category}
                summary={event.summary}
                recordedAt={event.recordedAt}
                open={expanded === event.seq}
                onToggle={() => setExpanded(expanded === event.seq ? null : event.seq)}
              >
                {event.triggeredBy && <TriggeredBy {...event.triggeredBy} />}
                <EventDetail
                  payload={event.payload}
                  nameById={names}
                  conversationNameById={convNames}
                  recordedAt={event.recordedAt}
                  source={event.source}
                />
              </LogEventRow>
            ))}
          </div>
          <Pager page={page} total={rows.length} onPage={setPage} />
        </>
      )}
    </section>
  );
}

/// A dim, clickable annotation linking back to the conversation turn that last touched this pass's
/// memory before it ran, shown at the head of the expanded detail. The annotation shows the
/// triggering turn's speaker and a truncated snippet of its text; clicking navigates to the
/// Conversation view pinned to that exact turn.
function TriggeredBy({
  turn,
  speaker,
  text,
  platform,
  scopePath,
}: {
  turn: string;
  speaker: string | null;
  text: string;
  platform: string;
  scopePath: string;
}) {
  const stream = useOptionalStream();
  const room = `${platform} · ${scopePath}`;
  const snippet = text.replace(/\s+/g, " ").trim();
  const label = speaker ? `after ${speaker}'s turn` : "after the agent's turn";
  const body = (
    <>
      {label}
      {" · "}
      <span className="italic">
        {snippet.length > 60 ? `“${snippet.slice(0, 60)}…”` : `“${snippet}”`}
      </span>
    </>
  );
  return (
    <div className="mb-2 text-ink-faint">
      {stream ? (
        <Link
          to={stream.link.conversation({ turn })}
          className="transition-colors hover:text-clay"
          title={`Open this turn in ${room}`}
        >
          {body}
        </Link>
      ) : (
        body
      )}
    </div>
  );
}

/// A pass group: a label and the background events it collects, ordered newest first.
interface PassGroup {
  id: string;
  label: string;
  events: BackgroundEvent[];
}

/// Classify a background event into its pass group by type.
function passGroupId(type: EventPayload["type"]): string {
  switch (type) {
    case "MemoryDescriptionRegenerated":
      return "description";
    case "EntryTemporalResolved":
    case "EntryTemporalResolveFailed":
      return "temporal-extraction";
    case "BeliefArbitrated":
      return "arbitration";
    case "LinksInferred":
      return "link-inference";
    case "EntriesConsolidated":
      return "consolidation";
    default:
      // Only ever called for BACKGROUND_TYPES members (`buildBackgroundEvents` filters to them
      // via `isBackgroundEvent`), so any other variant reaching here means BACKGROUND_TYPES and
      // this switch have drifted apart — add the new background type to the set and give it a
      // group above.
      throw new Error(`passGroupId: ${type} is not a background-pass type`);
  }
}

/// Group background events by pass type, preserving the display order of the groups and sorting each
/// group's events newest first (by recording time, with seq as a stable tiebreak). Only groups with
/// at least one event become subtabs.
function groupByPass(events: BackgroundEvent[]): PassGroup[] {
  const order = [
    "description",
    "temporal-extraction",
    "arbitration",
    "link-inference",
    "consolidation",
  ];
  const labels: Record<string, string> = {
    description: "description",
    "temporal-extraction": "temporal extraction",
    arbitration: "arbitration",
    "link-inference": "link inference",
    consolidation: "consolidation",
  };
  const byGroup = new Map<string, BackgroundEvent[]>();
  for (const event of events) {
    const id = passGroupId(event.type);
    let list = byGroup.get(id);
    if (!list) {
      list = [];
      byGroup.set(id, list);
    }
    list.push(event);
  }
  return order
    .filter((id) => byGroup.has(id))
    .map((id) => ({
      id,
      label: labels[id],
      events: [...byGroup.get(id)!].sort((a, b) => b.recordedAt - a.recordedAt || b.seq - a.seq),
    }));
}
