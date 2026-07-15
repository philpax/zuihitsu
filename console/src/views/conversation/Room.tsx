import { useContext, useState } from "react";

import type { Replica } from "../../lib/replica/replica.ts";
import { normalizeTurnRefs } from "../../lib/replica/replica.ts";
import { imprint } from "../../lib/api/operator.ts";
import { sendMessage } from "../../lib/api/participant.ts";
import { formatTokens } from "../../lib/format/format.ts";
import { Eyebrow } from "../../components/primitives.tsx";
import { Composer } from "./Composer.tsx";
import { Docked } from "./Docked.tsx";
import type { InFlightGeneration } from "../../lib/model/inflight.ts";
import { Transcript } from "./Transcript.tsx";
import { TurnMarkdown } from "./TurnMarkdown.tsx";
import { type Participation, ModelCalls } from "./conversationContexts.ts";
import { warmthAggregate } from "../../lib/model/contextDebug.ts";
import { type Channel, hasScopeChar } from "./channelUtilities.ts";
import { turnTokens } from "./turnUtilities.ts";
import { useFollowBottom } from "./useFollowBottom.ts";

/// One conversation, open: its header, sessions, and transcript, plus — live and at the head — a
/// composer routed to the room's authority (the imprint room writes `self`; the rest are ordinary
/// participant turns).
export function Room({
  replica,
  cursor,
  atHead,
  channel,
  inflight,
  participate,
  unknownTurn,
}: {
  replica: Replica;
  cursor: number;
  /// Whether the timeline cursor sits at the head: the transcript follows its own foot only here, so
  /// scrubbing history never drags the reader forward. Gates the composer too — you speak into the
  /// present, not into replayed history.
  atHead: boolean;
  channel: Channel;
  /// This room's in-flight generation (live mode only), rendered at the transcript's tail while
  /// the agent deliberates.
  inflight?: InFlightGeneration | null;
  participate?: Participation;
  /// A `?turn` deep link whose id resolved to no folded turn — surfaced as a quiet notice.
  unknownTurn?: string | null;
}) {
  const isOperator = channel.authority === "operator";
  const handle = participate?.sender.trim() ?? "";
  const handleScoped = hasScopeChar(handle);
  const { bySeq, denominators } = useContext(ModelCalls);
  const budget = denominators.budget;
  // The conversation's measured cache health: median warmth and total fresh-encoded tokens across
  // its calls, from the same reconstruction the per-call panels read.
  const warmth = warmthAggregate(
    [...bySeq.values()]
      .filter((call) => call.conversation === channel.conversation?.id)
      .map((call) => call.usage),
  );
  // The conversation's running cost, shown in the header: total generated (additive across turns) and
  // the peak context any turn reached (the high-water mark against the compaction budget — not a sum,
  // which would double-count the re-sent buffer).
  const convoTokens = (channel.conversation?.turns ?? []).reduce(
    (acc, turn) => {
      const { context, output } = turnTokens(turn, bySeq);
      return { peakContext: Math.max(acc.peakContext, context), output: acc.output + output };
    },
    { peakContext: 0, output: 0 },
  );
  const convoTowardCompaction =
    budget !== null && budget > 0 ? Math.round((convoTokens.peakContext / budget) * 100) : null;
  // True while a sent turn is in flight, so the conversation shows the agent at work.
  const [thinking, setThinking] = useState(false);
  // The just-sent turn, shown optimistically until the live tail folds the real one in — so the
  // message appears the instant it is sent rather than after the round-trip, and the thinking pulse
  // never sits against a conversation that does not yet show what was said. `baseline` is the turn
  // count at send; once the conversation grows past it, the real turn has landed and this is dropped.
  const [optimistic, setOptimistic] = useState<{ text: string; baseline: number } | null>(null);
  // A send whose wire outcome was `Deferred`: the message was delivered and recorded, but the
  // agent's model was unreachable, so no reply is coming for it — a quiet state, not an error.
  // `baseline` is the turn count at send; the marker clears once an agent turn lands past it (the
  // catch-up turn covered the deferred inbound) or the next send replaces it.
  const [deferred, setDeferred] = useState<{ baseline: number } | null>(null);

  async function onSend(text: string) {
    if (!participate) return;
    // The connector contract: a console URL must never reach the agent. The console is a connector,
    // so it converts any pasted turn deep-link into the canonical `[turn:<ulid>]` token here, before
    // the POST — the single send path for both authorities below (participant message and operator
    // imprint), so no console-originated message escapes normalization. The log, and every downstream
    // consumer including the agent's token-only resolver, then sees only ref syntax. The optimistic
    // echo shows the normalized text, matching the turn the live tail will fold in.
    const message = normalizeTurnRefs(text);
    const baseline = channel.conversation?.turns.length ?? 0;
    setOptimistic({ text: message, baseline });
    setDeferred(null);
    try {
      const response = isOperator
        ? await imprint(participate.connection, message)
        : await sendMessage(participate.connection, {
            locator: channel.locator,
            sender: handle,
            text: message,
            present: [handle],
          });
      if (response.outcome === "Deferred") setDeferred({ baseline });
    } catch (error) {
      setOptimistic(null); // the send failed — drop the optimistic turn (the composer restores the draft).
      throw error;
    }
  }

  // The deferral is covered once the agent speaks again in this room: its next turn replayed the
  // buffer, deferred inbounds included, so the marker would only restate what the reply shows.
  const deferredCovered =
    deferred !== null &&
    (channel.conversation?.turns ?? [])
      .slice(deferred.baseline)
      .some((turn) => turn.role === "Agent");

  // Follow the foot of the transcript while at the head, so everything that lands at the tail stays in
  // view — a new turn, a streamed reasoning or reply token, a committed deliberation step (a tool-call
  // notice), the optimistic echo, and the thinking pulse. Gated to `atHead`, so scrubbing history never
  // drags the reader forward; it holds in both frames, so a read-only eval run watched at its head
  // follows the tail too. The signal changes whenever the foot moves: `cursor` advances on every
  // committed event (new turns and deliberation steps alike), and the in-flight lengths grow with each
  // streamed token before that step commits — so the follow tracks token growth, not just whole turns.
  const inflightSignal = inflight
    ? `${inflight.step}:${inflight.phase}:${inflight.reasoning.length}:${inflight.reply.length}:${inflight.restarts}`
    : "";
  useFollowBottom(atHead, `${cursor}|${optimistic !== null}|${thinking}|${inflightSignal}`);

  return (
    <div className="flex w-full min-w-0 max-w-[60rem] flex-col">
      <header className="mb-4">
        <Eyebrow>
          {channel.label}
          {!isOperator && (
            <span className=" text-ink-faint">
              {" "}
              ({channel.locator.platform} · {channel.locator.scope_path})
            </span>
          )}
        </Eyebrow>
        {/* The locator addresses a real room; for the operator channel it just echoes the title. */}

        {convoTokens.peakContext + convoTokens.output > 0 && (
          <p className="mt-1 font-mono text-2xs text-ink-faint">
            {formatTokens(convoTokens.output)} generated · peak{" "}
            {formatTokens(convoTokens.peakContext)}
            {convoTowardCompaction !== null ? (
              <> · {convoTowardCompaction}% to compaction</>
            ) : (
              <> · budget unknown</>
            )}
            {warmth.median !== null && (
              <span title="Median measured cache warmth across this conversation's calls, and the total tokens the provider encoded fresh.">
                {" "}
                · {Math.round(warmth.median * 100)}% warm · {formatTokens(warmth.rePrefilled)} ↻
              </span>
            )}
          </p>
        )}
      </header>

      {unknownTurn && <UnknownTurnNotice />}

      {channel.conversation ? (
        <Transcript
          replica={replica}
          conversation={channel.conversation}
          cursor={cursor}
          inflight={inflight}
        />
      ) : (
        <p className="text-sm text-ink-faint">
          {isOperator
            ? "Introduce yourself to begin the interview."
            : "No messages yet — say hello."}
        </p>
      )}

      {optimistic !== null && (channel.conversation?.turns.length ?? 0) <= optimistic.baseline && (
        <OptimisticTurn
          speaker={replica.participantName(channel.locator.platform, handle)}
          text={optimistic.text}
        />
      )}

      {deferred !== null && !deferredCovered && <DeferredNotice />}

      {thinking && <ThinkingIndicator />}

      {/* The composer floats in the workspace's bottom dock, so you can start typing from anywhere
          in the transcript. It mirrors the view's transcript-and-rooms grid so the writing line
          sits exactly under the transcript column, with the spacer standing in for the room list. */}
      {participate && (
        <Docked>
          <div className="pb-2.5 pt-2 md:grid md:grid-cols-[1fr_12rem] md:gap-8">
            <div className="w-full min-w-0 max-w-[60rem]">
              {atHead ? (
                <Composer
                  onSend={onSend}
                  onPendingChange={setThinking}
                  disabled={!isOperator && (handle.length === 0 || handleScoped)}
                  disabledHint={
                    handleScoped
                      ? "The handle should be a bare name, not a memory path."
                      : "Set who you are to start."
                  }
                  placeholder={
                    isOperator
                      ? "Speak to the agent as the operator…"
                      : `Message ${channel.label} as ${handle || "…"}`
                  }
                />
              ) : (
                <p className="mb-2 rounded-sm border border-line bg-paper px-3 py-2 text-center font-mono text-xs text-ink-faint">
                  viewing history · return to the head of the timeline to speak
                </p>
              )}
            </div>
          </div>
        </Docked>
      )}
    </div>
  );
}

/// A sent message was delivered and durably recorded, but the agent's model was unreachable, so
/// the reply is deferred — a quiet state marker where the reply would land, not an error: the
/// agent replays the buffer on its next turn and catches up then. Faint ink, no accent — the
/// message is safe, and the header banner already carries the degraded-backend signal.
function DeferredNotice() {
  return (
    <div className="mt-5 flex items-center gap-2 text-ink-faint">
      <span className="inline-flex h-1.5 w-1.5 rounded-full border border-line-strong" />
      <span className="font-mono text-2xs uppercase tracking-widest">
        delivered — the agent will catch up when its model returns
      </span>
    </div>
  );
}

/// A `?turn` deep link whose id no folded turn carries — unknown, mistyped, or a moment past the
/// timeline cursor. A quiet inline notice in the deferred marker's register (faint ink, no accent),
/// not an error: the transcript below is intact, only the pointer failed to land.
function UnknownTurnNotice() {
  return (
    <div className="mb-4 flex items-center gap-2 text-ink-faint">
      <span className="inline-flex h-1.5 w-1.5 rounded-full border border-line-strong" />
      <span className="font-mono text-2xs uppercase tracking-widest">
        that turn link points nowhere in view — an unknown id, or a moment past the timeline cursor
      </span>
    </div>
  );
}

/// The agent is composing a reply — a sage pulse where the next turn will land, shown between the
/// transcript and the composer while a sent turn is in flight.
function ThinkingIndicator() {
  return (
    <div className="mt-5 flex items-center gap-2 text-sage">
      <span className="relative flex h-1.5 w-1.5">
        <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-sage opacity-60" />
        <span className="relative inline-flex h-1.5 w-1.5 rounded-full bg-sage" />
      </span>
      <span className="font-mono text-2xs uppercase tracking-widest">the agent is thinking…</span>
    </div>
  );
}

/// The just-sent turn, echoed at the head of the transcript while it is in flight — dimmed and marked
/// "sending" so it reads as not-yet-confirmed, matching a participant turn's shape so the live tail's
/// real turn replaces it without a visible jump.
function OptimisticTurn({ speaker, text }: { speaker: string; text: string }) {
  return (
    <div className="border-t border-line/70 py-4 opacity-55 sm:py-5">
      <div className="mb-1.5 flex items-baseline gap-2">
        <span className="font-mono text-2xs font-medium uppercase tracking-widest text-clay">
          {speaker}
        </span>
        <span className="ml-auto shrink-0 font-mono text-2xs text-ink-faint">sending…</span>
      </div>
      <TurnMarkdown text={text} softBreaks />
    </div>
  );
}
