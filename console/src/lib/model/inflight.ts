import type { Event } from "@zuihitsu/wire/types/Event.ts";
import type { TurnProgress } from "@zuihitsu/wire/types/TurnProgress.ts";

/// An in-flight generation's accumulated text — the ephemeral deliberation a viewer watches arrive
/// token by token before the committed `ModelCalled` supersedes it. One shared model for every
/// surface that watches tokens: the live agent console and the eval deep-dive fold the identical
/// `TurnProgress` frames through the identical logic, so "watching a generation" means one thing.
export interface InFlightGeneration {
  turnId: string;
  step: number;
  phase: TurnProgress["phase"];
  reasoning: string;
  reply: string;
  /// How many attempts were discarded and re-driven (a transient mid-stream failure each) —
  /// rendered as a quiet "attempt N" so a restart reads as a retry, not a glitch.
  restarts: number;
  /// True once the step's `ModelCalled` has committed: the streamed text has a durable successor,
  /// so its display yields — but the accumulation itself must survive, because it is what holds the
  /// pending turn's place in the transcript until the view's cursor catches up with the commit (the
  /// eval deep-dive folds its replica asynchronously, so the cursor lags the event by one refold).
  /// Deleting on commit would evict the pending turn during that window, remounting the turn and
  /// losing its view state. The next step's first frame starts a fresh, unsuperseded accumulation.
  superseded: boolean;
}

/// Fold one frame into a conversation's accumulation. Always returns a fresh object: the views'
/// inputs must change identity per frame, or the React Compiler's memoisation (correctly) bails on
/// the same reference and the panel freezes on the step's first token. A frame for a new turn,
/// step, or phase starts a fresh accumulation; a `restart` frame voids the step's text and counts
/// the retry; an `abandoned` frame — the generation died with no durable successor, e.g. a
/// deferral — returns `undefined`, telling the caller to drop the accumulation outright.
export function foldFrame(
  current: InFlightGeneration | undefined,
  frame: TurnProgress,
): InFlightGeneration | undefined {
  const base =
    current &&
    current.turnId === frame.turn_id &&
    current.step === frame.step &&
    current.phase === frame.phase
      ? current
      : {
          turnId: frame.turn_id,
          step: frame.step,
          phase: frame.phase,
          reasoning: "",
          reply: "",
          restarts: 0,
          superseded: false,
        };
  switch (frame.kind) {
    case "reasoning":
      return { ...base, reasoning: base.reasoning + frame.text };
    case "reply":
      return { ...base, reply: base.reply + frame.text };
    case "restart":
      return { ...base, reasoning: "", reply: "", restarts: base.restarts + 1 };
    case "abandoned":
      return undefined;
  }
}

/// The conversation whose in-flight accumulation a committed event supersedes, or `null`. A
/// `ModelCalled` makes the step's generation durable; the agent's `ConversationTurn` is the turn's
/// true end. (A deferred turn commits neither — its generation ends via the `abandoned` progress
/// frame in `foldFrame`, not through an event.)
export function supersededConversation(event: Event): string | null {
  const payload = event.payload as { type?: string; conversation?: string; role?: string };
  if (!payload.conversation) return null;
  if (payload.type === "ModelCalled") return payload.conversation;
  if (payload.type === "ConversationTurn" && payload.role === "Agent") return payload.conversation;
  return null;
}

/// How a conversation's accumulation responds to the committed event that supersedes it (an event
/// `supersededConversation` matched). A mid-turn `ModelCalled` only *marks* the accumulation: the
/// object must keep holding the pending turn's transcript slot until the cursor catches up with the
/// commit (see `superseded`). The agent's `ConversationTurn` ends a turn that is materialised under
/// its own key by its recorded deliberation, so there the accumulation is dropped (`undefined`)
/// outright.
export function supersede(
  current: InFlightGeneration,
  event: Event,
): InFlightGeneration | undefined {
  const payload = event.payload as { type?: string };
  if (payload.type === "ModelCalled") {
    return current.superseded ? current : { ...current, superseded: true };
  }
  return undefined;
}
