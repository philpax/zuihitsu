import { useContext, useState } from "react";

import type { Replica } from "../../lib/replica/replica.ts";
import { normalizeTurnRefs } from "../../lib/view/turnRefs.ts";
import { normalizeMemRefs } from "../../lib/view/memRefs.ts";
import { imprint } from "../../lib/api/operator.ts";
import { consoleOrigins } from "../../lib/api/http.ts";
import { DIRECT_PLATFORM, sendMessage } from "../../lib/api/participant.ts";
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
import { useStream } from "../../lib/nav/useStreamLocation.ts";
import { ScrollContainer } from "../../lib/nav/scrollContainer.ts";
import { useTranscriptScroll } from "./useTranscriptScroll.ts";

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
  // The console is the operator's loopback `direct` interface: the server scopes every message it
  // sends to `direct`, so composing into a room that belongs to another platform's connector would
  // silently target a `direct` room of the same scope path instead of the real one. Such rooms are
  // view-only here — the console is not a stand-in for that platform's connector.
  const foreignRoom = !isOperator && channel.locator.platform !== DIRECT_PLATFORM;
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
    // The platform connector contract: a console URL must never reach the agent. The console is a platform connector,
    // so it converts any pasted deep-link into its canonical reference token here, before the POST — a
    // turn deep-link via the wasm token normalizer, and a State-view deep-link via route matching in
    // the nav layer, which resolves the handle the URL routes by and mints the token. This is the single send path for both authorities below (participant message and
    // operator imprint), so no console-originated message escapes normalization. The log, and every
    // downstream consumer including the agent's token-only resolver, then sees only ref syntax. The
    // optimistic echo shows the normalized text, matching the turn the live tail will fold in.
    //
    // Only a deep link on an origin the console owns (its own, or its configured backend) is rewritten,
    // so a foreign URL that merely shares the console's path shape stays prose rather than being replaced
    // by a token.
    const origins = consoleOrigins(participate.connection);
    const message = normalizeMemRefs(normalizeTurnRefs(text, origins), replica, origins);
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
      // `Superseded` deliberately gets no marker: a newer batch's turn answers this one with
      // everything in context, and that successor's reply arrives through the live tail like any
      // other agent turn — there is nothing quiet to explain, unlike a deferral.
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
  // Live (a composer is present) windows the transcript, opens on the tail, and offers a jump-to-latest
  // pill; a read-only eval run keeps the whole transcript and only follows its foot at the head. A
  // `?turn` deep link to this room opens the window around that turn rather than the tail.
  const { search } = useStream();
  const turns = channel.conversation?.turns ?? [];
  const focusTurn = search.turn ?? null;
  const focusIndex = focusTurn ? turns.findIndex((turn) => turn.turnId === focusTurn) : -1;
  // The scrolling well the transcript lives in, from the workspace. The scroll hook drives this
  // element (not the window), so the document stays fixed while the transcript scrolls between the
  // nav and the docked composer.
  const container = useContext(ScrollContainer);
  const scroll = useTranscriptScroll({
    mode: participate ? "live" : "review",
    active: atHead,
    total: turns.length,
    focusIndex: focusIndex >= 0 ? focusIndex : null,
    footSignal: `${cursor}|${optimistic !== null}|${thinking}|${inflightSignal}`,
    inflightActive: inflight != null,
    container,
  });

  return (
    <div className="flex w-full max-w-240 min-w-0 flex-col">
      <header className="mb-4">
        <Eyebrow>
          {channel.label}
          {!isOperator && (
            <span className="text-ink-faint">
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
          window={scroll.window}
          topRef={scroll.topRef}
          bottomRef={scroll.bottomRef}
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

      {scroll.showJump && <JumpToLatest count={scroll.newCount} onClick={scroll.jumpToLatest} />}

      {/* The composer floats in the workspace's bottom dock, so you can start typing from anywhere
          in the transcript. It mirrors the view's transcript-and-rooms grid so the writing line
          sits exactly under the transcript column, with the spacer standing in for the room list. */}
      {participate && (
        <Docked>
          <div className="pt-2 pb-2.5 md:grid md:grid-cols-[1fr_12rem] md:gap-8">
            <div className="w-full max-w-240 min-w-0">
              {atHead ? (
                <Composer
                  onSend={onSend}
                  onPendingChange={setThinking}
                  disabled={foreignRoom || (!isOperator && (handle.length === 0 || handleScoped))}
                  disabledHint={
                    foreignRoom
                      ? `View-only — ${channel.locator.platform} rooms belong to that platform's connector, not the console.`
                      : handleScoped
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
      <span className="inline-flex size-1.5 rounded-full border border-line-strong" />
      <span className="font-mono text-2xs tracking-widest uppercase">
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
      <span className="inline-flex size-1.5 rounded-full border border-line-strong" />
      <span className="font-mono text-2xs tracking-widest uppercase">
        that turn link points nowhere in view — an unknown id, or a moment past the timeline cursor
      </span>
    </div>
  );
}

/// A floating jump-to-latest indicator, shown when the reader has scrolled up off the foot while new
/// activity lands at the tail — a quiet pill in the transcript's register (faint ink on paper, a
/// hairline border), with the count of unseen turns in clay. Clicking it snaps the window back to the
/// tail and re-pins the follow. Sticky to the foot of the scrolling well, so it floats just above the
/// docked composer (which is fixed chrome below the well); the wrapper passes clicks through its empty
/// margins so only the pill itself is interactive.
function JumpToLatest({ count, onClick }: { count: number; onClick: () => void }) {
  return (
    <div className="pointer-events-none sticky bottom-4 z-20 flex justify-center">
      <button
        onClick={onClick}
        className="pointer-events-auto flex items-center gap-2 rounded-full border border-line bg-paper/95 px-3.5 py-1.5 font-mono text-2xs text-ink-soft shadow-sm backdrop-blur-sm transition-colors hover:text-ink"
      >
        {count > 0 && (
          <span className="text-clay">
            {count} new message{count > 1 ? "s" : ""}
          </span>
        )}
        <span className="tracking-widest uppercase">jump to latest ↓</span>
      </button>
    </div>
  );
}

/// The agent is composing a reply — a sage pulse where the next turn will land, shown between the
/// transcript and the composer while a sent turn is in flight.
function ThinkingIndicator() {
  return (
    <div className="mt-5 flex items-center gap-2 text-sage">
      <span className="relative flex size-1.5">
        <span className="absolute inline-flex size-full animate-ping rounded-full bg-sage opacity-60" />
        <span className="relative inline-flex size-1.5 rounded-full bg-sage" />
      </span>
      <span className="font-mono text-2xs tracking-widest uppercase">the agent is thinking…</span>
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
        <span className="font-mono text-2xs font-medium tracking-widest text-clay uppercase">
          {speaker}
        </span>
        <span className="ml-auto shrink-0 font-mono text-2xs text-ink-faint">sending…</span>
      </div>
      <TurnMarkdown text={text} softBreaks />
    </div>
  );
}
