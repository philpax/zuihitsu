import type { Event } from "@zuihitsu/wire/types/Event.ts";
import type { EventPayload } from "@zuihitsu/wire/types/EventPayload.ts";
import type { Replica } from "../lib/replica/replica.ts";
import { nameById } from "../lib/model/labels.ts";
import { buildBackgroundEvents, type BackgroundEvent } from "../lib/model/conversation.ts";
import { EventRow } from "../components/EventRow.tsx";
import { conversationNameById } from "../lib/model/conversationNameById.ts";
import { Eyebrow } from "../components/primitives.tsx";

/// The Background view: the background passes' (describer, temporal extraction, belief
/// arbitration, link-inference) log-only audit events, collected from the run's event stream and
/// grouped by pass type. These
/// events carry no conversation or turn attribution — they run asynchronously, potentially long
/// after the turn that inspired them — so they surface here as a top-level timeline alongside the
/// Conversation view rather than mis-attributed to a turn or silently dropped. Each row is the
/// one-line summary by default and expands, in place, into the same specialized viewer the Events
/// tab uses, with a "triggered by" annotation linking back to the conversation turn that last
/// touched the memory before the pass ran.
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

  return (
    <section>
      <div className="mb-3 flex items-baseline justify-between gap-4">
        <Eyebrow>{background.length} background events</Eyebrow>
        <Eyebrow>
          seq 1 – {cursor}
          {cursor < events.length ? ` of ${events.length}` : ""}
        </Eyebrow>
      </div>

      {background.length === 0 ? (
        <p className="font-mono text-2xs text-ink-faint">no background passes recorded</p>
      ) : (
        <div className="flex flex-col gap-6">
          {groups.map((group) => (
            <div key={group.id}>
              <Eyebrow className="mb-2">
                {group.label} ({group.events.length})
              </Eyebrow>
              <ul className="flex flex-col gap-0.5">
                {group.events.map((event) => (
                  <EventRow
                    key={event.seq}
                    row={event}
                    nameById={names}
                    conversationNameById={convNames}
                    triggeredBy={event.triggeredBy}
                  />
                ))}
              </ul>
            </div>
          ))}
        </div>
      )}
    </section>
  );
}

/// A pass group: a label and the background events it collects, ordered by seq.
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
    default:
      // Only ever called for BACKGROUND_TYPES members (`buildBackgroundEvents` filters to them), so
      // any other variant reaching here means BACKGROUND_TYPES and this switch have drifted apart —
      // add the new background type to the set and give it a group above.
      throw new Error(`passGroupId: ${type} is not a background-pass type`);
  }
}

/// Group background events by pass type, preserving the display order of the groups and sorting
/// events within each group by seq (they are already seq-sorted from `buildBackgroundEvents`, but
/// the explicit sort guards against any drift).
function groupByPass(events: BackgroundEvent[]): PassGroup[] {
  const order = ["description", "temporal-extraction", "arbitration", "link-inference"];
  const labels: Record<string, string> = {
    description: "description",
    "temporal-extraction": "temporal extraction",
    arbitration: "arbitration",
    "link-inference": "link inference",
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
      events: [...byGroup.get(id)!].sort((a, b) => a.seq - b.seq),
    }));
}
