import type { EventPayload } from "../types/EventPayload.ts";

/// A coarse grouping of event kinds, for a calm colour rhythm in the log: memory writes, the link
/// graph, conversation flow, the agent's deliberation, session/room lifecycle, and infrastructure.
export type EventCategory =
  | "memory"
  | "link"
  | "conversation"
  | "deliberation"
  | "lifecycle"
  | "infra";

export function eventCategory(type: EventPayload["type"]): EventCategory {
  switch (type) {
    case "MemoryCreated":
    case "MemoryRenamed":
    case "MemoryDeleted":
    case "MemoryContentAppended":
    case "MemorySuperseded":
    case "EntryTemporalResolved":
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

/// A concise, human-readable one-line summary of an event, resolving ids to handles where it can.
export function eventSummary(payload: EventPayload, nameById: Map<string, string>): string {
  const ref = (id: string) => nameById.get(id) ?? shortId(id);

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
        ? "Error" in payload.terminal_cause
          ? `error: ${payload.terminal_cause.Error}`
          : `aborted: ${payload.terminal_cause.Aborted}`
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
