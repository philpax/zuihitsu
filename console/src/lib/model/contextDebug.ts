import type { Event } from "../../types/Event.ts";
import { type CacheVerdict, deriveCachePaths } from "./cachePath.ts";
import {
  type ContextDenominators,
  type ModelInteraction,
  buildInteractions,
  contextDenominatorsAt,
} from "./interactions.ts";
import { type CallAttribution, attributeTokens } from "./tokenAttribution.ts";

/// Everything the context debugger derives from the log at a cursor: the reconstructed calls, a
/// cache verdict and token attribution per call, and the denominators the sizes read against.
/// Verdicts compare consecutive calls **within a conversation** — a call in one room says nothing
/// about the cache state of another.
export interface ContextDebug {
  bySeq: Map<number, ModelInteraction>;
  verdictBySeq: Map<number, CacheVerdict>;
  attributionBySeq: Map<number, CallAttribution>;
  /// The denominators in effect at each call's own seq — a mid-run settings change (the eval
  /// scenarios retune the budget freely) must not repaint earlier calls.
  denominatorsBySeq: Map<number, ContextDenominators>;
  /// The denominators at the view cursor, for conversation-level chrome.
  denominators: ContextDenominators;
}

export function deriveContextDebug(events: Event[], cursor: number): ContextDebug {
  const calls = buildInteractions(events, cursor);
  const bySeq = new Map(calls.map((call) => [call.seq, call]));

  // Chains compare consecutive calls within a conversation *and* phase: a Synthesis call is a
  // separate structured request, and letting one interleave into the Step chain would read as a
  // spurious system change and break the measured-delta ladder for the step after it.
  const callsByChain = new Map<string, ModelInteraction[]>();
  for (const call of calls) {
    const key = `${call.conversation} ${call.phase}`;
    const chain = callsByChain.get(key);
    if (chain) chain.push(call);
    else callsByChain.set(key, [call]);
  }

  const seamsByConversation = new Map<string, number[]>();
  for (const event of events) {
    if (event.seq > cursor || event.payload.type !== "SessionStarted") continue;
    const seams = seamsByConversation.get(event.payload.conversation);
    if (seams) seams.push(event.seq);
    else seamsByConversation.set(event.payload.conversation, [event.seq]);
  }

  const verdictBySeq = new Map<number, CacheVerdict>();
  const attributionBySeq = new Map<number, CallAttribution>();
  for (const chain of callsByChain.values()) {
    const conversation = chain[0].conversation;
    const verdicts = deriveCachePaths(chain, seamsByConversation.get(conversation) ?? []);
    const attributions = attributeTokens(chain, verdicts);
    chain.forEach((call, index) => {
      verdictBySeq.set(call.seq, verdicts[index]);
      attributionBySeq.set(call.seq, attributions[index]);
    });
  }

  // One pass over the ConfigSets, then a pointer walk over the seq-ordered calls — resolving each
  // call's denominators by rescanning the whole log would be O(calls × events).
  const configSets: Array<{ seq: number; denominators: ContextDenominators }> = [];
  for (const event of events) {
    if (event.seq > cursor || event.payload.type !== "ConfigSet") continue;
    const compaction = event.payload.settings.compaction;
    configSets.push({
      seq: event.seq,
      denominators: {
        budget: compaction.token_budget,
        contextLength: compaction.context_length ?? null,
      },
    });
  }
  configSets.sort((a, b) => a.seq - b.seq);
  const denominatorsBySeq = new Map<number, ContextDenominators>();
  let atConfig = -1;
  for (const call of calls) {
    while (atConfig + 1 < configSets.length && configSets[atConfig + 1].seq <= call.seq) {
      atConfig += 1;
    }
    denominatorsBySeq.set(
      call.seq,
      atConfig >= 0 ? configSets[atConfig].denominators : { budget: null, contextLength: null },
    );
  }

  return {
    bySeq,
    verdictBySeq,
    attributionBySeq,
    denominatorsBySeq,
    denominators: contextDenominatorsAt(events, cursor),
  };
}
