import type { EventPayload } from "@zuihitsu/wire/types/EventPayload.ts";
import type { EventSource } from "@zuihitsu/wire/types/EventSource.ts";
import type { LinkSource } from "@zuihitsu/wire/types/LinkSource.ts";
import { terminalCauseLabel } from "./labels.ts";

/// The authoring authorities offered as an author filter in the Events view — genesis first, then
/// the agent's turns, the operator's console actions, and the system's background work. A platform
/// connector is not offered as a standalone filter (it is a tagged variant, not a bare string), but
/// `sourceLabel` renders it when it appears in an event.
export const EVENT_SOURCES: EventSource[] = ["Bootstrap", "Agent", "Operator", "Orchestration"];

/// The human-facing label for an event's authoring authority — the envelope `source`. Lowercased
/// against the mono type the log speaks; the enum's own words otherwise.
export function sourceLabel(source: EventSource): string {
  if (source === "Bootstrap") return "genesis";
  if (source === "Agent") return "agent";
  if (source === "Operator") return "operator";
  if (source === "Orchestration") return "system";
  return `platform connector: ${source.PlatformConnector}`;
}

/// The human-facing label for a link's provenance — the agent's own edge, an operator's console
/// assertion, an inferred edge, or a platform connector's structural link, which names the platform
/// so an audit can tell which one authored it.
export function linkSourceLabel(source: LinkSource): string {
  if (typeof source === "string") return source.toLowerCase();
  return `platform connector: ${source.PlatformConnector}`;
}

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
    case "EntryRetracted":
    case "EntryTemporalResolved":
    case "EntryTemporalResolveFailed":
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
    case "LinksInferred":
      return "link";
    case "ConversationTurn":
    case "TurnSuperseded":
    case "ParticipantJoined":
    case "ParticipantIdentified":
      return "conversation";
    case "ModelCalled":
    case "ModelCallAborted":
    case "AmbientRecallSurfaced":
    case "LuaExecuted":
      return "deliberation";
    case "ConversationStarted":
    case "ConversationEnded":
    case "SessionStarted":
    case "SessionEnded":
    case "ScheduledJobFired":
    case "ScheduledItemSurfaced":
      return "lifecycle";
    case "GenesisCompleted":
    case "ConfigSet":
    case "PromptTemplateRegistered":
    case "EmbeddingModelChanged":
    case "DescribePassCompleted":
    case "ClassPrimaryDesignated":
      return "infra";
    default: {
      // The console's new-event tripwire (CONTRIBUTING: surface new state in the frontend). This
      // switch is deliberately exhaustive: a new EventPayload variant regenerates into the wire
      // union on the next `cargo build -p zuihitsu`, and typecheck fails here until the variant is
      // categorised — and while you are here, give it a summary below and, if it renders, a viewer
      // or transcript surface.
      const unhandled: never = type;
      return unhandled;
    }
  }
}

/// The event types the background passes emit — log-only audit records with no conversation or
/// turn attribution. They surface in the Background view rather than the Conversation transcript.
export const BACKGROUND_TYPES = new Set<EventPayload["type"]>([
  "MemoryDescriptionRegenerated",
  "EntryTemporalResolved",
  "EntryTemporalResolveFailed",
  "BeliefArbitrated",
  "LinksInferred",
]);

/// Whether an event type is produced by a background pass (the describer, temporal extraction,
/// belief arbitration, or link-inference), and so belongs in the Background view.
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
    case "EntryTemporalResolveFailed":
    case "EntryDescriptionMirrored":
    case "MemoryDescriptionRegenerated":
    case "MemoryVolatilitySet":
      return payload.id === memoryId;
    case "MemoryContentAppended":
      return payload.id === memoryId;
    case "EntryRetracted":
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
    case "AmbientRecallSurfaced":
      return payload.hits.some((hit) => hit.memory === memoryId);
    // Schema, lifecycle, and deliberation events with no single memory subject — they never key a
    // per-memory history jump.
    case "TagCreated":
    case "TagDescriptionChanged":
    case "LinkTypeRegistered":
    case "TurnSuperseded":
    case "ModelCalled":
    case "ModelCallAborted":
    case "ConversationEnded":
    case "SessionEnded":
    case "GenesisCompleted":
    case "ConfigSet":
    case "PromptTemplateRegistered":
    case "EmbeddingModelChanged":
    case "DescribePassCompleted":
    case "ClassPrimaryDesignated":
      return false;
    default: {
      // Exhaustive — the new-event tripwire lives on eventCategory; categorise there first, then here.
      const unhandled: never = payload;
      return unhandled;
    }
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
    case "EntryRetracted":
      return `${ref(payload.memory)} — an entry retracted`;
    case "EntryTemporalResolved":
      return `${ref(payload.id)} — time resolved`;
    case "EntryTemporalResolveFailed":
      return `${ref(payload.id)} — time resolution dropped`;
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
    case "ModelCallAborted":
      return `attempt ${payload.attempt} discarded — ${payload.cause}`;
    case "AmbientRecallSurfaced":
      return `ambient recall — ${payload.hits.map((hit) => ref(hit.memory)).join(", ") || "nothing"}`;
    case "TurnSuperseded":
      return "a turn superseded before its reply";
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
    case "TagDescriptionChanged":
      return `#${payload.name} — ${payload.new_description}`;
    case "ParticipantJoined":
      return `${ref(payload.participant)} joined`;
    case "ScheduledItemSurfaced":
      return `${ref(payload.memory)} surfaced`;
    case "SessionEnded":
      return payload.cause ? `session ended — ${payload.cause.toLowerCase()}` : "session ended";
    case "ConversationEnded":
      return "conversation ended";
    case "EmbeddingModelChanged":
      return `${payload.from} → ${payload.to}`;
    case "DescribePassCompleted":
      return `described ${payload.memories.length} ${payload.memories.length === 1 ? "memory" : "memories"}`;
    case "ClassPrimaryDesignated":
      return `${ref(payload.memory)} — ${payload.designated ? "primary" : "no longer primary"}`;
    default: {
      // Exhaustive — the new-event tripwire lives on eventCategory; categorise there first, then here.
      const unhandled: never = payload;
      return unhandled;
    }
  }
}

function quote(text: string): string {
  const trimmed = text.replace(/\s+/g, " ").trim();
  return trimmed.length > 80 ? `“${trimmed.slice(0, 80)}…”` : `“${trimmed}”`;
}

function shortId(id: string): string {
  return id.length > 10 ? `${id.slice(0, 4)}…${id.slice(-4)}` : id;
}
