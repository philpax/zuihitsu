import type { Event } from "../../types/Event.ts";
import type { Brief } from "../../types/Brief.ts";
import type { Completion } from "../../types/Completion.ts";
import type { EventPayload } from "../../types/EventPayload.ts";
import type { Initiation } from "../../types/Initiation.ts";
import type { ModelPhase } from "../../types/ModelPhase.ts";
import type { TerminalCause } from "../../types/TerminalCause.ts";
import type { TurnRole } from "../../types/TurnRole.ts";
import { type EventCategory, eventCategory, eventSummary, isBackgroundEvent } from "./events.ts";

/// The graph-mutating events that result from an agent turn's Lua — what the turn actually *did*.
/// They carry no `turn_id`, but are committed with the block that produced them, so they are
/// attributed to the turn whose deliberation is in flight when they appear in `seq` order.
const OUTCOME_TYPES = new Set<EventPayload["type"]>([
  "MemoryCreated",
  "MemoryRenamed",
  "MemoryDeleted",
  "MemoryContentAppended",
  "MemorySuperseded",
  "MemoryVolatilitySet",
  "MemoryDescriptionRegenerated",
  "BeliefArbitrated",
  "EntryTemporalResolved",
  "TagCreated",
  "TagDescriptionChanged",
  "TagAppliedToMemory",
  "TagRemovedFromMemory",
  "LinkTypeRegistered",
  "LinkCreated",
  "LinkRemoved",
]);

/// The Conversation view's model, reconstructed from a run's event stream: each durable room, its
/// sessions (with the brief frozen at start), and the ordered turns. Every agent turn carries its
/// deliberation — the reasoning steps and Lua blocks that produced it — interleaved in `seq` order,
/// which is "what was the agent thinking" made literal.

export interface ConversationModel {
  id: string;
  platform: string;
  scopePath: string;
  contextName: string | null;
  /// The room's `context/*` memory id, for re-deriving the brief.
  contextMemory: string | null;
  sessions: SessionModel[];
  turns: TurnModel[];
}

export interface SessionModel {
  id: string;
  /// The seq the session opened at, for interleaving its brief with the turns.
  seq: number;
  brief: string;
  startedAt: number;
  participants: string[];
  /// The present set as memory ids (the names above are resolved from these).
  participantIds: string[];
  /// True when the session opened by re-segmenting a prior one (compaction) rather than fresh — it
  /// carries `seeded_from_turn`, so the transcript can mark a continuity cut.
  compaction: boolean;
}

export interface TurnModel {
  turnId: string;
  seq: number;
  /// The wall-clock time the turn was committed (the `ConversationTurn` event's `recorded_at`), `0`
  /// until that event is seen — a turn assembled only from in-flight deliberation has no time yet.
  recordedAt: number;
  role: TurnRole;
  text: string;
  speaker: string | null;
  initiation: Initiation;
  deliberation: DeliberationStep[];
  /// What the turn produced: the graph-mutating events its Lua committed (writes, links, tags,
  /// arbitrations), in order — the consequence of the deliberation above.
  outcomes: TurnOutcome[];
  /// True when this turn is the speaker's first appearance and they were not in the opening present
  /// set — a mid-conversation entrance, surfaced so a participant does not just materialize.
  entrance: boolean;
  /// The memory a scheduled wake-up surfaced from, when this `Initiated` turn was the agent speaking
  /// to a fired reminder rather than responding (spec §Agent-initiated speech).
  wakeup: string | null;
  /// The structured join-brief a mid-session join's `system` turn carries (spec §Mid-conversation
  /// joins): the same content `text` holds as rendered markup, kept as data so the transcript renders
  /// a proper entrance treatment rather than the raw markup. `null` for every other turn.
  brief: Brief | null;
}

/// One graph-mutating event a turn produced, summarized for the transcript and carrying the full
/// payload so a row can expand into the same specialized viewer the Events tab uses.
export interface TurnOutcome {
  seq: number;
  recordedAt: number;
  type: EventPayload["type"];
  category: EventCategory;
  summary: string;
  payload: EventPayload;
}

export type DeliberationStep =
  | {
      kind: "model";
      seq: number;
      phase: ModelPhase;
      reasoning: string | null;
      completion: Completion;
      finishReason: string | null;
      durationMs: number;
    }
  | {
      kind: "lua";
      seq: number;
      script: string;
      result: string | null;
      terminalCause: TerminalCause | null;
      durationMs: number;
    };

/// Fold a run's events into its conversations, resolving participant and room ids to handles through
/// `nameById`. A single pass, tolerant of order: an agent turn's deliberation events precede its
/// `ConversationTurn` in the log, so turns are created lazily and ordered by their canonical turn seq.
export function buildConversations(
  events: Event[],
  nameById: Map<string, string>,
): ConversationModel[] {
  const conversations = new Map<string, ConversationModel>();
  const turns = new Map<string, TurnModel>();

  function conversation(id: string): ConversationModel {
    let model = conversations.get(id);
    if (!model) {
      model = {
        id,
        platform: "",
        scopePath: "",
        contextName: null,
        contextMemory: null,
        sessions: [],
        turns: [],
      };
      conversations.set(id, model);
    }
    return model;
  }

  function turn(conversationId: string, turnId: string, seq: number): TurnModel {
    let model = turns.get(turnId);
    if (!model) {
      model = {
        turnId,
        seq,
        recordedAt: 0,
        role: "Agent",
        text: "",
        speaker: null,
        initiation: "Responding",
        deliberation: [],
        outcomes: [],
        entrance: false,
        wakeup: null,
        brief: null,
      };
      turns.set(turnId, model);
      conversation(conversationId).turns.push(model);
    }
    return model;
  }

  function name(id: string | null): string | null {
    if (id === null) return null;
    return nameById.get(id) ?? id;
  }

  // The turn whose deliberation is in flight, used to attribute the writes it commits — its Lua's
  // side-effect events carry no turn_id but appear contiguously in `seq` while it runs. The lock set
  // each turn's blocks touched, and the candidate outcomes, are kept for a second pass that attributes
  // a write only when its memory is in the turn's touched set — so between-turn orchestration (room
  // minting, first-contact stubs), which shares no lock set, is excluded.
  let currentTurnId: string | null = null;
  // A wake-up surfaces just before the Initiated turn it raises; hold it until that turn claims it.
  let pendingWakeup: string | null = null;
  const touchedByTurn = new Map<string, Set<string>>();
  const candidates: Array<{
    turnId: string;
    seq: number;
    recordedAt: number;
    payload: EventPayload;
  }> = [];

  for (const event of [...events].sort((a, b) => a.seq - b.seq)) {
    const payload = event.payload;
    switch (payload.type) {
      case "ConversationStarted": {
        const model = conversation(payload.id);
        model.platform = payload.locator.platform;
        model.scopePath = payload.locator.scope_path;
        model.contextName = name(payload.context_memory);
        model.contextMemory = payload.context_memory;
        // A new room's eager setup (its context memory, first-contact stubs) is not a turn's doing.
        currentTurnId = null;
        break;
      }
      case "SessionStarted": {
        conversation(payload.conversation).sessions.push({
          id: payload.id,
          seq: event.seq,
          brief: payload.brief,
          startedAt: payload.started_at,
          participants: payload.participants.map((id) => name(id) ?? id),
          participantIds: payload.participants,
          compaction: payload.seeded_from_turn !== null,
        });
        break;
      }
      case "ConversationTurn": {
        const model = turn(payload.conversation, payload.turn_id, event.seq);
        model.seq = event.seq;
        model.recordedAt = event.recorded_at;
        model.role = payload.role;
        model.text = payload.text;
        model.speaker = name(payload.participant);
        model.initiation = payload.initiation;
        model.brief = payload.brief;
        // Outcomes belong to the agent's response cycle; an inbound or system turn closes the prior
        // one so its post-reply synthesis attributes correctly and later setup does not.
        currentTurnId = payload.role === "Agent" ? payload.turn_id : null;
        if (payload.initiation === "Initiated" && pendingWakeup) {
          model.wakeup = pendingWakeup;
          pendingWakeup = null;
        }
        break;
      }
      case "ScheduledItemSurfaced": {
        pendingWakeup = name(payload.memory);
        break;
      }
      case "ModelCalled": {
        turn(payload.conversation, payload.turn_id, event.seq).deliberation.push({
          kind: "model",
          seq: event.seq,
          phase: payload.phase,
          reasoning: payload.reasoning,
          completion: payload.completion,
          finishReason: payload.finish_reason,
          durationMs: Number(payload.duration_ms),
        });
        currentTurnId = payload.turn_id;
        break;
      }
      case "LuaExecuted": {
        turn(payload.conversation, payload.turn_id, event.seq).deliberation.push({
          kind: "lua",
          seq: event.seq,
          script: payload.script,
          result: payload.result,
          terminalCause: payload.terminal_cause,
          durationMs: Number(payload.duration_ms),
        });
        currentTurnId = payload.turn_id;
        let touched = touchedByTurn.get(payload.turn_id);
        if (!touched) {
          touched = new Set();
          touchedByTurn.set(payload.turn_id, touched);
        }
        for (const id of payload.touched) touched.add(id);
        break;
      }
      default: {
        if (currentTurnId && OUTCOME_TYPES.has(payload.type)) {
          candidates.push({
            turnId: currentTurnId,
            seq: event.seq,
            recordedAt: event.recorded_at,
            payload,
          });
        }
      }
    }
  }

  // Attribute outcomes: a write belongs to a turn only if the turn's blocks touched its memory (or
  // it is a schema event — a tag/relation registration — with no memory to key on). Candidates are
  // in seq order, so outcomes land in order.
  for (const { turnId, seq, recordedAt, payload } of candidates) {
    const turnModel = turns.get(turnId);
    if (!turnModel) continue;
    const ids = outcomeMemoryIds(payload);
    const touched = touchedByTurn.get(turnId);
    if (ids.length === 0 || ids.some((id) => touched?.has(id) ?? false)) {
      turnModel.outcomes.push({
        seq,
        recordedAt,
        type: payload.type,
        category: eventCategory(payload.type),
        summary: eventSummary(payload, nameById),
        payload,
      });
    }
  }

  for (const model of conversations.values()) {
    model.turns.sort((a, b) => a.seq - b.seq);
    for (const t of model.turns) t.deliberation.sort((a, b) => a.seq - b.seq);
    // A participant speaking for the first time, when they were not in the session's opening present
    // set, is a mid-conversation entrance — mark it so the transcript shows them arriving rather than
    // simply appearing (the brief, frozen at session start, does not yet know them).
    const present = new Set(model.sessions[0]?.participants ?? []);
    for (const turn of model.turns) {
      if (turn.role === "Participant" && turn.speaker && !present.has(turn.speaker)) {
        turn.entrance = true;
        present.add(turn.speaker);
      }
    }
  }
  return [...conversations.values()];
}

/// The memory ids an outcome event targets — used to check it against the turn's touched set. A
/// schema event (a tag or relation registration) targets no memory and returns empty, attributed by
/// being inside a turn at all.
function outcomeMemoryIds(payload: EventPayload): string[] {
  switch (payload.type) {
    case "MemoryCreated":
    case "MemoryRenamed":
    case "MemoryDeleted":
    case "MemoryContentAppended":
    case "MemorySuperseded":
    case "MemoryVolatilitySet":
    case "MemoryDescriptionRegenerated":
    case "EntryTemporalResolved":
      return [payload.id];
    case "BeliefArbitrated":
      return [payload.memory];
    case "TagAppliedToMemory":
    case "TagRemovedFromMemory":
      return [payload.memory];
    case "LinkCreated":
    case "LinkRemoved":
      return [payload.from, payload.to];
    default:
      return [];
  }
}

/// One background-pass event (a description regeneration, a belief arbitration, an inferred link
/// set, or a merge adjudication), summarized for the Background view and carrying the full payload
/// so a row can expand into the same specialized viewer the Events tab uses — the same shape as
/// [`TurnOutcome`].
export interface BackgroundEvent {
  seq: number;
  recordedAt: number;
  type: EventPayload["type"];
  category: EventCategory;
  summary: string;
  payload: EventPayload;
  /// The conversation turn that last touched this memory before the background pass ran — the
  /// temporal link from the async pass back to the conversation that triggered it. A best-effort
  /// bridge, not a precise causal link: the pass processes all memories changed since its cursor,
  /// so the "last touch" is the most likely trigger. `null` when no preceding `LuaExecuted` touched
  /// the memory (e.g., a genesis-seeded memory). The locator fields build the `?room=` query param
  /// that navigates to the conversation in the Conversation view.
  triggeredBy: {
    speaker: string | null;
    text: string;
    platform: string;
    scopePath: string;
  } | null;
}

/// The memory ids a background-pass event targets — mirrors [`outcomeMemoryIds`] but for the
/// background types. `MemoryDescriptionRegenerated` uses `payload.id` (the memory being described);
/// `BeliefArbitrated` and `LinksInferred` use `payload.memory`; `MergeAdjudicated` uses both
/// `payload.from` and `payload.to`.
function backgroundMemoryIds(payload: EventPayload): string[] {
  switch (payload.type) {
    case "MemoryDescriptionRegenerated":
      return [payload.id];
    case "BeliefArbitrated":
    case "LinksInferred":
      return [payload.memory];
    case "MergeAdjudicated":
      return [payload.from, payload.to];
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
  // The locator for each conversation, so a `?room=` param can be built from a conversation id.
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
          // For `MergeAdjudicated` (two ids), use whichever was touched most recently.
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
