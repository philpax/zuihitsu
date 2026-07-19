import type { Event } from "@zuihitsu/wire/types/Event.ts";
import type { EventPayload } from "@zuihitsu/wire/types/EventPayload.ts";
import type { EventSource } from "@zuihitsu/wire/types/EventSource.ts";
import { type EventCategory, eventCategory, eventSummary, isBackgroundEvent } from "./events.ts";

/// One background-pass event (a description regeneration, a temporal resolution or its recorded
/// drop, a belief arbitration, or an inferred link set), summarized for the Background view and
/// carrying the full payload so a row can expand into the same specialized viewer the Events tab
/// uses — the same shape as [`TurnOutcome`].
export interface BackgroundEvent {
  seq: number;
  recordedAt: number;
  /// The envelope's authoring authority, shown as faint provenance in the expanded row.
  source: EventSource;
  type: EventPayload["type"];
  category: EventCategory;
  summary: string;
  payload: EventPayload;
  /// The conversation turn that last touched this memory before the background pass ran — the
  /// temporal link from the async pass back to the conversation that triggered it. A best-effort
  /// bridge, not a precise causal link: the pass processes all memories changed since its cursor,
  /// so the "last touch" is the most likely trigger. `null` when no preceding `LuaExecuted` touched
  /// the memory (e.g., a genesis-seeded memory). The locator fields build the room segment that
  /// navigates to the conversation in the Conversation view.
  triggeredBy: {
    speaker: string | null;
    text: string;
    platform: string;
    scopePath: string;
  } | null;
}

/// The memory ids a background-pass event targets — mirrors [`outcomeMemoryIds`] but for the
/// background types. `MemoryDescriptionRegenerated` and the temporal-resolution pair use
/// `payload.id` (the memory whose entry the pass touched); `BeliefArbitrated` and `LinksInferred`
/// use `payload.memory`.
function backgroundMemoryIds(payload: EventPayload): string[] {
  switch (payload.type) {
    case "MemoryDescriptionRegenerated":
    case "EntryTemporalResolved":
    case "EntryTemporalResolveFailed":
      return [payload.id];
    case "BeliefArbitrated":
    case "LinksInferred":
      return [payload.memory];
    default:
      return [];
  }
}

/// Collect the background-pass events from a run's log, up to the cursor, each linked back to the
/// conversation turn that last touched its memory before the pass ran. A single pass over the
/// events (in seq order) builds the `memoryToLastTurn` map as it goes, so each background event sees
/// the last touch before it — not the last touch overall. The `cursor` filter is required:
/// [`StreamWorkspace`](../components/StreamWorkspace.tsx) passes the full `events` array to every
/// view, and each view is responsible for filtering to the timeline cursor.
export function buildBackgroundEvents(
  events: Event[],
  nameById: Map<string, string>,
  cursor: number,
): BackgroundEvent[] {
  const result: BackgroundEvent[] = [];

  // The most recent `LuaExecuted` that touched each memory, at the current point in the seq walk.
  const memoryToLastTurn = new Map<string, { seq: number; turnId: string; conversation: string }>();
  // The turn for each `turn_id`, for resolving the speaker and text from its `ConversationTurn`.
  const turnById = new Map<
    string,
    { speaker: string | null; text: string; conversation: string }
  >();
  // The locator for each conversation, so a room segment can be built from a conversation id.
  const conversationLocator = new Map<string, { platform: string; scopePath: string }>();

  for (const event of [...events].sort((a, b) => a.seq - b.seq)) {
    if (event.seq > cursor) break;
    const payload = event.payload;

    switch (payload.type) {
      case "ConversationStarted": {
        conversationLocator.set(payload.id, {
          platform: payload.locator.platform,
          scopePath: payload.locator.scope_path,
        });
        break;
      }
      case "ConversationTurn": {
        const speaker = payload.participant
          ? (nameById.get(payload.participant) ?? payload.participant)
          : null;
        turnById.set(payload.turn_id, {
          conversation: payload.conversation,
          speaker,
          text: payload.text,
        });
        break;
      }
      case "LuaExecuted": {
        for (const id of payload.touched) {
          memoryToLastTurn.set(id, {
            seq: event.seq,
            turnId: payload.turn_id,
            conversation: payload.conversation,
          });
        }
        break;
      }
      default: {
        if (isBackgroundEvent(payload.type)) {
          const ids = backgroundMemoryIds(payload);
          // With several candidate ids, use whichever was touched most recently.
          let best: { seq: number; turnId: string; conversation: string } | null = null;
          for (const id of ids) {
            const candidate = memoryToLastTurn.get(id);
            if (candidate && (!best || candidate.seq > best.seq)) best = candidate;
          }
          const turn = best ? (turnById.get(best.turnId) ?? null) : null;
          const locator = turn ? (conversationLocator.get(turn.conversation) ?? null) : null;
          result.push({
            seq: event.seq,
            recordedAt: event.recorded_at,
            source: event.source,
            type: payload.type,
            category: eventCategory(payload.type),
            summary: eventSummary(payload, nameById),
            payload,
            triggeredBy:
              turn && locator
                ? {
                    speaker: turn.speaker,
                    text: turn.text,
                    platform: locator.platform,
                    scopePath: locator.scopePath,
                  }
                : null,
          });
        }
        break;
      }
    }
  }

  return result.sort((a, b) => a.seq - b.seq);
}
