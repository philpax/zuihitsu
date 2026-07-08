import type { EventPayload } from "../../types/EventPayload.ts";
import { terminalCauseLabel } from "./labels.ts";

/// A coarse grouping of event kinds, for a calm colour rhythm in the log: memory writes, the link
/// graph, conversation flow, the agent's deliberation, session/room lifecycle, and infrastructure.
export type EventCategory =
  | "memory"
  | "link"
  | "conversation"
  | "deliberation"
  | "lifecycle"
  | "infra";

/// The text color each category lends its event-type label — the restrained rhythm shared by the
/// Events log and the per-turn outcomes in the transcript.
export const CATEGORY_COLOR: Record<EventCategory, string> = {
  memory: "text-clay",
  link: "text-sage",
  conversation: "text-ink",
  deliberation: "text-ink-soft",
  lifecycle: "text-sage",
  infra: "text-ink-faint",
};

export function eventCategory(type: EventPayload["type"]): EventCategory {
  switch (type) {
    case "MemoryCreated":
    case "MemoryRenamed":
    case "MemoryDeleted":
    case "MemoryContentAppended":
    case "MemorySuperseded":
    case "EntryTemporalResolved":
    case "EntryDescriptionMirrored":
    case "MemoryDescriptionRegenerated":
    case "BeliefArbitrated":
    case "MemoryVolatilitySet":
    case "TagCreated":
    case "TagDescriptionChanged":
    case "TagAppliedToMemory":
    case "TagRemovedFromMemory":
      return "memory";
    case "LinkTypeRegistered":
    case "LinkCreated":
    case "LinkRemoved":
    case "MergeProposed":
    case "MergeAdjudicated":
    case "LinksInferred":
      return "link";
    case "ConversationTurn":
    case "ParticipantJoined":
    case "ParticipantIdentified":
      return "conversation";
    case "ModelCalled":
    case "LuaExecuted":
      return "deliberation";
    case "ConversationStarted":
    case "ConversationEnded":
    case "SessionStarted":
    case "SessionEnded":
    case "ScheduledJobFired":
    case "ScheduledItemSurfaced":
      return "lifecycle";
    default:
      return "infra";
  }
}

/// The event types the background passes emit — log-only audit records with no conversation or
/// turn attribution. They surface in the Background view rather than the Conversation transcript.
export const BACKGROUND_TYPES = new Set<EventPayload["type"]>([
  "MemoryDescriptionRegenerated",
  "BeliefArbitrated",
  "LinksInferred",
  "MergeAdjudicated",
]);

/// Whether an event type is produced by a background pass (the describer, adjudicator,
/// link-inference, or merge-adjudicator), and so belongs in the Background view.
export function isBackgroundEvent(type: EventPayload["type"]): boolean {
  return BACKGROUND_TYPES.has(type);
}

/// Resolve a memory id to its handle, falling back to an abbreviated id when it is not in the map —
/// how the Events log and the per-event detail name the ids they reference.
export function refName(id: string, nameById: Map<string, string>): string {
  return nameById.get(id) ?? shortId(id);
}

/// Whether an event references `memoryId` — backs the State view's "events touching this memory" jump.
/// Covers the memory's own mutations (create, append, supersede, rename, delete, description,
/// volatility, arbitration), its tags, links from either end, scheduled occurrences, the `told_in`
/// room an aside was scoped to, a block that touched it, and the conversation/identity references that
/// name it.
export function eventTouchesMemory(payload: EventPayload, memoryId: string): boolean {
  switch (payload.type) {
    case "MemoryCreated":
    case "MemoryRenamed":
    case "MemoryDeleted":
    case "MemorySuperseded":
    case "EntryTemporalResolved":
    case "EntryDescriptionMirrored":
    case "MemoryDescriptionRegenerated":
    case "MemoryVolatilitySet":
      return payload.id === memoryId;
    case "MemoryContentAppended":
      return payload.id === memoryId || payload.told_in === memoryId;
    case "ScheduledJobFired":
    case "ScheduledItemSurfaced":
    case "BeliefArbitrated":
    case "LinksInferred":
    case "TagAppliedToMemory":
    case "TagRemovedFromMemory":
    case "ParticipantIdentified":
      return payload.memory === memoryId;
    case "LinkCreated":
    case "LinkRemoved":
    case "MergeProposed":
    case "MergeAdjudicated":
      return payload.from === memoryId || payload.to === memoryId;
    case "ConversationStarted":
      return payload.context_memory === memoryId;
    case "SessionStarted":
      return payload.participants.includes(memoryId);
    case "ParticipantJoined":
      return payload.participant === memoryId;
    case "ConversationTurn":
      return payload.participant === memoryId;
    case "LuaExecuted":
      return payload.touched.includes(memoryId);
    default:
      return false;
  }
}

/// A concise, human-readable one-line summary of an event, resolving ids to handles where it can.
export function eventSummary(payload: EventPayload, nameById: Map<string, string>): string {
  const ref = (id: string) => refName(id, nameById);

  switch (payload.type) {
    case "MemoryCreated":
      return payload.name;
    case "MemoryRenamed":
      return `${payload.old_name} → ${payload.new_name}`;
    case "MemoryDeleted":
      return ref(payload.id);
    case "MemoryContentAppended":
      return `${ref(payload.id)} — ${quote(payload.text)}`;
    case "MemorySuperseded":
      return `${ref(payload.id)} — an entry replaced`;
    case "EntryTemporalResolved":
      return `${ref(payload.id)} — time resolved`;
    case "EntryDescriptionMirrored":
      return `${ref(payload.id)} — description mirror`;
    case "MemoryDescriptionRegenerated":
      return `${ref(payload.id)} — ${quote(payload.new_text)}`;
    case "BeliefArbitrated":
      return `${ref(payload.memory)} — ${quote(payload.resolution.statement)}`;
    case "MemoryVolatilitySet":
      return `${ref(payload.id)} — ${payload.volatility}`;
    case "TagCreated":
      return `#${payload.name} — ${payload.description}`;
    case "TagAppliedToMemory":
      return `${ref(payload.memory)} #${payload.tag}`;
    case "TagRemovedFromMemory":
      return `${ref(payload.memory)} −#${payload.tag}`;
    case "LinkCreated":
      return `${ref(payload.from)} ${payload.relation} ${ref(payload.to)}`;
    case "LinkRemoved":
      return `${ref(payload.from)} −${payload.relation} ${ref(payload.to)}`;
    case "MergeProposed":
      return `${ref(payload.from)} ⇄ ${ref(payload.to)} — merge proposed`;
    case "MergeAdjudicated":
      return `${ref(payload.from)} ⇄ ${ref(payload.to)} — ${
        payload.accepted ? "merged" : "merge refused"
      }: ${payload.rationale}`;
    case "LinksInferred": {
      const links = payload.result.links.map((l) => `${l.relation} → ${l.target}`).join(", ");
      const coined = payload.result.new_relations.map((r) => r.name).join(", ");
      const detail = [links, coined && `coined: ${coined}`].filter(Boolean).join("; ");
      return `${ref(payload.memory)} — ${detail || "no relationships found"}`;
    }
    case "LinkTypeRegistered":
      return `${payload.name} / ${payload.inverse}`;
    case "ConversationStarted":
      return ref(payload.context_memory);
    case "SessionStarted":
      return `${payload.participants.map(ref).join(", ") || "no one"} present`;
    case "ConversationTurn":
      return `${payload.role.toLowerCase()} — ${quote(payload.text)}`;
    case "ModelCalled":
      return `${payload.phase.toLowerCase()} call`;
    case "LuaExecuted":
      return payload.terminal_cause
        ? terminalCauseLabel(payload.terminal_cause)
        : quote(payload.script);
    case "ParticipantIdentified":
      return `${ref(payload.memory)} @${payload.platform}`;
    case "ScheduledJobFired":
      return `${ref(payload.memory)} fired`;
    case "ConfigSet":
      return "settings snapshot";
    case "PromptTemplateRegistered":
      return `${payload.name} v${payload.version}`;
    case "GenesisCompleted":
      return "genesis";
    default:
      return "";
  }
}

function quote(text: string): string {
  const trimmed = text.replace(/\s+/g, " ").trim();
  return trimmed.length > 80 ? `“${trimmed.slice(0, 80)}…”` : `“${trimmed}”`;
}

function shortId(id: string): string {
  return id.length > 10 ? `${id.slice(0, 4)}…${id.slice(-4)}` : id;
}
