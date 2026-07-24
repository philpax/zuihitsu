import type { Event } from "@zuihitsu/wire/types/Event.ts";
import type { Brief } from "@zuihitsu/wire/types/Brief.ts";
import type { Completion } from "@zuihitsu/wire/types/Completion.ts";
import type { EventPayload } from "@zuihitsu/wire/types/EventPayload.ts";
import type { EventSource } from "@zuihitsu/wire/types/EventSource.ts";
import type { Initiation } from "@zuihitsu/wire/types/Initiation.ts";
import type { ModelPhase } from "@zuihitsu/wire/types/ModelPhase.ts";
import type { SessionEndCause } from "@zuihitsu/wire/types/SessionEndCause.ts";
import type { TerminalCause } from "@zuihitsu/wire/types/TerminalCause.ts";
import type { TurnRole } from "@zuihitsu/wire/types/TurnRole.ts";
import { type EventCategory, eventCategory, eventSummary } from "./events.ts";

// Re-export the background-pass model so existing importers (`BackgroundView` and others) keep a
// single entry point — the split is structural, not a public API change.
export { type BackgroundEvent, buildBackgroundEvents } from "./background.ts";

/// The graph-mutating events that result from an agent turn's Lua — what the turn actually *did*.
/// They carry no `turn_id`, but are committed with the block that produced them, so they are
/// attributed to the turn whose deliberation is in flight when they appear in `seq` order.
const OUTCOME_TYPES = new Set<EventPayload["type"]>([
  "MemoryCreated",
  "MemoryRenamed",
  "MemoryDeleted",
  "MemoryContentAppended",
  "MemorySuperseded",
  "EntryRetracted",
  "EntryAttested",
  "AttestationRetracted",
  "MemoryVolatilitySet",
  "MemoryDescriptionRegenerated",
  "BeliefArbitrated",
  "EntryTemporalResolved",
  "EntryDescriptionMirrored",
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
  /// The initiating speakers — the participants whose inbound message opened the session, whom the
  /// brief guarantees a full block. Resolved names; a subset of `participants`. Empty for an
  /// agent-initiated open or a session recorded before capture.
  speakers: string[];
  /// The initiating speakers as memory ids (the names above are resolved from these).
  speakerIds: string[];
  /// True when the session opened by re-segmenting a prior one rather than fresh — it carries
  /// `seeded_from_turn`, so the transcript can mark a continuity cut. The specific seam kind is the
  /// *previous* session's `endCause`, which the divider reads to label it.
  seededFromTail: boolean;
  /// Why this session ended, set when its `SessionEnded` arrives — `null` while still open, or for a
  /// close recorded before the cause was captured. The next session's divider reads it to name the
  /// seam (a compaction cut, an idle timeout, or a recovery close).
  endCause: SessionEndCause | null;
  /// The recorded working-set memory ids the session opened with, or `null` for a session recorded
  /// before capture — the raw payload lacks the key, which is distinct from a genuinely empty set.
  workingSet: string[] | null;
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
  /// The recorded participant id behind `speaker` — the platform stub the connector resolved. The
  /// view canonicalizes it to the class primary for display, dimming this actually-used handle.
  speakerId: string | null;
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
  /// True when this agent turn was produced by the checkpoint/end-of-session flush (its `produced_by`
  /// names the `Flush` template) — an internal bookkeeping turn that writes working state to memory
  /// and is delivered to no participant. Surfaced so the transcript marks it as internal at a glance.
  checkpoint: boolean;
}

/// One graph-mutating event a turn produced, summarized for the transcript and carrying the full
/// payload so a row can expand into the same specialized viewer the Events tab uses.
export interface TurnOutcome {
  seq: number;
  recordedAt: number;
  /// The envelope's authoring authority, shown as faint provenance in the expanded row.
  source: EventSource;
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
      /// A discarded streaming attempt: a transient mid-generation failure the retry wrapper
      /// re-drove. The partials are what was thrown away — the successful attempt follows as its
      /// own "model" step.
      kind: "aborted";
      seq: number;
      phase: ModelPhase;
      attempt: number;
      cause: string;
      partialReasoning: string;
      partialReply: string;
    }
  | {
      kind: "lua";
      seq: number;
      script: string;
      result: string | null;
      terminalCause: TerminalCause | null;
      durationMs: number;
    }
  | {
      /// The pre-turn ambient recall hint: memories the frozen brief did not carry, surfaced by the
      /// lexical pass and injected as a system message before the model generated. The `text` is the
      /// exact hint the model read; `memories` are the surfaced memory ids.
      kind: "ambient";
      seq: number;
      text: string;
      memories: string[];
    }
  | {
      /// The supersession seam marker: the turn was overtaken by a newer message before its reply
      /// was sent, so its committed steps stand with no answer beneath them. The `text` is the exact
      /// system hint the successor turn's buffer replays.
      kind: "superseded";
      seq: number;
      text: string;
    };

/// The shape a turn materialises through. A turn is built up incrementally — the fold creates it at
/// its first deliberation event and the `ConversationTurn` commit completes it (`recordedAt` stays
/// `0` until then; a deferred turn keeps this shape for good). The live transcript uses the same
/// constructor for a turn that so far exists only as streamed tokens, so an in-progress turn is an
/// ordinary `TurnModel` early in the one lifecycle every consumer already handles, not a parallel
/// pending type.
export function emptyTurn(turnId: string, seq: number): TurnModel {
  return {
    turnId,
    seq,
    recordedAt: 0,
    role: "Agent",
    text: "",
    speaker: null,
    speakerId: null,
    initiation: "Responding",
    deliberation: [],
    outcomes: [],
    entrance: false,
    wakeup: null,
    brief: null,
    checkpoint: false,
  };
}

/// Fold a run's events into its conversations, resolving participant and room ids to handles through
/// `nameById`. A single pass, tolerant of order: an agent turn's deliberation events precede its
/// `ConversationTurn` in the log, so turns are created lazily and ordered by their canonical turn seq.
export function buildConversations(
  events: Event[],
  nameById: Map<string, string>,
  liveConversationIds: Set<string>,
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
      model = emptyTurn(turnId, seq);
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
    source: EventSource;
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
          speakers: payload.initiators.map((id) => name(id) ?? id),
          speakerIds: payload.initiators,
          seededFromTail: payload.seeded_from_turn !== null,
          endCause: null,
          // Serde defaults an absent key to an empty array before the typed payload reaches us, so
          // the pre-capture distinction must come from the raw JSON key's presence.
          workingSet: "working_set" in payload ? payload.working_set : null,
        });
        break;
      }
      case "SessionEnded": {
        // Record the close cause on the ending session, so the next session's divider can name the
        // seam. A close for a session not in view (folded from before this window) is a no-op.
        const ended = conversation(payload.conversation).sessions.find(
          (session) => session.id === payload.id,
        );
        if (ended) ended.endCause = payload.cause;
        break;
      }
      case "ConversationTurn": {
        const model = turn(payload.conversation, payload.turn_id, event.seq);
        model.seq = event.seq;
        model.recordedAt = event.recorded_at;
        model.role = payload.role;
        model.text = payload.text;
        model.speaker = name(payload.participant);
        model.speakerId = payload.participant;
        model.initiation = payload.initiation;
        model.brief = payload.brief;
        model.checkpoint = payload.produced_by?.template_name === "flush";
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
      case "ModelCallAborted": {
        turn(payload.conversation, payload.turn_id, event.seq).deliberation.push({
          kind: "aborted",
          seq: event.seq,
          phase: payload.phase,
          attempt: payload.attempt,
          cause: payload.cause,
          partialReasoning: payload.partial_reasoning,
          partialReply: payload.partial_reply,
        });
        currentTurnId = payload.turn_id;
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
      case "AmbientRecallSurfaced": {
        // The hint sets up the answering turn — appended before the model's own steps and keyed to
        // the same turn id — so it leads the turn's deliberation.
        turn(payload.conversation, payload.turn_id, event.seq).deliberation.push({
          kind: "ambient",
          seq: event.seq,
          text: payload.text,
          memories: payload.hits.map((hit) => hit.memory),
        });
        currentTurnId = payload.turn_id;
        break;
      }
      case "TurnSuperseded": {
        // The marker closes the dead turn's deliberation: the steps above it are the committed
        // record, and the marker states that no reply followed.
        turn(payload.conversation, payload.turn_id, event.seq).deliberation.push({
          kind: "superseded",
          seq: event.seq,
          text: payload.text,
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
        currentTurnId = payload.turn_id;
        let touched = touchedByTurn.get(payload.turn_id);
        if (!touched) {
          touched = new Set();
          touchedByTurn.set(payload.turn_id, touched);
        }
        for (const id of payload.touched) touched.add(id);
        break;
      }
      // Not transcript material: a graph-mutating outcome an agent's Lua committed is gathered as a
      // candidate here (gated by OUTCOME_TYPES and the in-flight turn) and attributed to that turn in
      // the second pass below; every other kind is folded elsewhere (the Background and Events views)
      // and ignored here. Enumerated so a new EventPayload variant trips the never-check rather than
      // being silently dropped.
      case "MemoryCreated":
      case "MemoryRenamed":
      case "MemoryDeleted":
      case "MemoryContentAppended":
      case "MemorySuperseded":
      case "EntriesConsolidated":
      case "EntryRetracted":
      case "EntryAttested":
      case "AttestationRetracted":
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
      case "LinkTypeRegistered":
      case "LinkCreated":
      case "LinkRemoved":
      case "MergeProposed":
      case "LinksInferred":
      case "ParticipantJoined":
      case "ParticipantIdentified":
      case "ConversationEnded":
      case "ScheduledJobFired":
      case "GenesisCompleted":
      case "ConfigSet":
      case "PromptTemplateRegistered":
      case "EmbeddingModelChanged":
      case "DescribePassCompleted":
      case "ClassPrimaryDesignated": {
        if (currentTurnId && OUTCOME_TYPES.has(payload.type)) {
          candidates.push({
            turnId: currentTurnId,
            seq: event.seq,
            recordedAt: event.recorded_at,
            source: event.source,
            payload,
          });
        }
        break;
      }
      default: {
        // Exhaustive — the new-event tripwire lives on eventCategory; categorise there first, then here.
        const unhandled: never = payload;
        void unhandled;
      }
    }
  }

  // Attribute outcomes: a write belongs to a turn only if the turn's blocks touched its memory (or
  // it is a schema event — a tag/relation registration — with no memory to key on). Candidates are
  // in seq order, so outcomes land in order.
  for (const { turnId, seq, recordedAt, source, payload } of candidates) {
    const turnModel = turns.get(turnId);
    if (!turnModel) continue;
    const ids = outcomeMemoryIds(payload);
    const touched = touchedByTurn.get(turnId);
    if (ids.length === 0 || ids.some((id) => touched?.has(id) ?? false)) {
      turnModel.outcomes.push({
        seq,
        recordedAt,
        source,
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
  // The graph is the single authority on which conversations exist: `replica.conversations()` drops a
  // conversation whose room memory was deleted, so keep only those it still holds. The transcript above
  // is folded from the event stream (the events are the content); existence is not re-derived here.
  return [...conversations.values()].filter((model) => liveConversationIds.has(model.id));
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
    case "EntriesConsolidated":
    case "MemoryVolatilitySet":
    case "MemoryDescriptionRegenerated":
    case "EntryTemporalResolved":
    case "EntryDescriptionMirrored":
      return [payload.id];
    case "EntryRetracted":
    case "EntryAttested":
    case "AttestationRetracted":
    case "BeliefArbitrated":
      return [payload.memory];
    case "TagAppliedToMemory":
    case "TagRemovedFromMemory":
      return [payload.memory];
    case "LinkCreated":
    case "LinkRemoved":
      return [payload.from, payload.to];
    // Schema outcomes — a tag or relation registration — target no memory: attributed by being
    // inside a turn at all. They are OUTCOME_TYPES members, so they reach here legitimately.
    case "TagCreated":
    case "TagDescriptionChanged":
    case "LinkTypeRegistered":
      return [];
    default:
      // Only ever called for OUTCOME_TYPES members (the second-pass candidates are gated by the
      // set), so any other variant reaching here means OUTCOME_TYPES and this switch have drifted
      // apart — add the new outcome type to the set and give it a case above.
      throw new Error(`outcomeMemoryIds: ${payload.type} is not an outcome type`);
  }
}
