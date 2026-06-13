import type { Event } from "../types/Event.ts";
import type { Completion } from "../types/Completion.ts";
import type { Initiation } from "../types/Initiation.ts";
import type { ModelPhase } from "../types/ModelPhase.ts";
import type { TerminalCause } from "../types/TerminalCause.ts";
import type { TurnRole } from "../types/TurnRole.ts";

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
  brief: string;
  startedAt: number;
  participants: string[];
  /// The present set as memory ids (the names above are resolved from these).
  participantIds: string[];
}

export interface TurnModel {
  turnId: string;
  seq: number;
  role: TurnRole;
  text: string;
  speaker: string | null;
  initiation: Initiation;
  deliberation: DeliberationStep[];
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
        role: "Agent",
        text: "",
        speaker: null,
        initiation: "Responding",
        deliberation: [],
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

  for (const event of events) {
    const payload = event.payload;
    switch (payload.type) {
      case "ConversationStarted": {
        const model = conversation(payload.id);
        model.platform = payload.locator.platform;
        model.scopePath = payload.locator.scope_path;
        model.contextName = name(payload.context_memory);
        model.contextMemory = payload.context_memory;
        break;
      }
      case "SessionStarted": {
        conversation(payload.conversation).sessions.push({
          id: payload.id,
          brief: payload.brief,
          startedAt: payload.started_at,
          participants: payload.participants.map((id) => name(id) ?? id),
          participantIds: payload.participants,
        });
        break;
      }
      case "ConversationTurn": {
        const model = turn(payload.conversation, payload.turn_id, event.seq);
        model.seq = event.seq;
        model.role = payload.role;
        model.text = payload.text;
        model.speaker = name(payload.participant);
        model.initiation = payload.initiation;
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
        break;
      }
    }
  }

  for (const model of conversations.values()) {
    model.turns.sort((a, b) => a.seq - b.seq);
    for (const t of model.turns) t.deliberation.sort((a, b) => a.seq - b.seq);
  }
  return [...conversations.values()];
}
