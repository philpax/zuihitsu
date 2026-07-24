import { useContext, useEffect, useRef, useState } from "react";
import { motion } from "motion/react";

import type { Event } from "@zuihitsu/wire/types/Event.ts";
import type { TurnModel } from "../../lib/model/conversation.ts";
import { formatDateTime, formatTime } from "../../lib/format/format.ts";
import { Disclosure, LabeledDivider } from "../../components/primitives.tsx";
import { EventDetail } from "../../components/EventDetail.tsx";
import { Link } from "../../lib/nav/history.tsx";
import { useStream } from "../../lib/nav/useStreamLocation.ts";
import { CallContext } from "./CallContext.tsx";
import { OutcomeList } from "./OutcomeList.tsx";
import { TurnMarkdown } from "./TurnMarkdown.tsx";
import { ConversationNames, EventsBySeq, ModelCalls, Names } from "./conversationContexts.ts";
import { turnTokens, linkedClass } from "./turnUtilities.ts";
import { JoinBriefTurn } from "./JoinBrief.tsx";
import { Deliberation } from "./Deliberation.tsx";
import type { InFlightGeneration } from "../../lib/model/inflight.ts";

export function TurnItem({
  turn,
  fresh,
  roomKey,
  inflight,
}: {
  turn: TurnModel;
  fresh: boolean;
  roomKey: string;
  /// This turn's in-flight generation (live mode): streamed into the deliberation collapsible.
  inflight?: InFlightGeneration | null;
}) {
  const { bySeq } = useContext(ModelCalls);
  const tokens = turnTokens(turn, bySeq);
  // The turn's final Step-phase call — the conversational context the turn ended with, footing the
  // turn with the same display each deliberation step carries. Synthesis calls are excluded: their
  // prompts are separate structured requests, not the conversation's context.
  const lastCallSeq = [...turn.deliberation]
    .reverse()
    .find((step) => step.kind === "model" && step.phase === "Step")?.seq;
  // The deep-linked turn (`?turn=<id>`) announces itself: scrolled into view once and washed in
  // fading sage, so a pasted link lands the reader on the moment it points at.
  const linked = useStream().search.turn === turn.turnId;
  const itemRef = useRef<HTMLLIElement>(null);
  useEffect(() => {
    if (linked) itemRef.current?.scrollIntoView({ block: "center", behavior: "smooth" });
  }, [linked]);
  // A turn that streamed in after the view opened fades and lifts into place; the initial ones do not.
  const enter = fresh
    ? {
        initial: { opacity: 0, y: 6 },
        animate: { opacity: 1, y: 0 },
        transition: { duration: 0.35, ease: [0.32, 0.72, 0, 1] as const },
      }
    : {};
  if (turn.role === "System") {
    // A join carries a structured brief: draw the entrance as a labelled seam with a disclosure into
    // the pretty-printed brief, rather than surfacing the raw markup `text`. A system turn without a
    // brief (a wake-up surface) keeps the plain centered line.
    if (turn.brief) {
      return (
        <JoinBriefTurn
          turn={turn}
          roomKey={roomKey}
          linked={linked}
          enter={enter}
          itemRef={itemRef}
        />
      );
    }
    return (
      <motion.li
        ref={itemRef}
        className={"flex items-baseline justify-center gap-2 py-3" + linkedClass(linked)}
        {...enter}
      >
        <span className="font-mono text-2xs text-ink-faint">{turn.text || "(system)"}</span>
        {turn.recordedAt > 0 && (
          <TurnTimeAnchor roomKey={roomKey} turnId={turn.turnId} recordedAt={turn.recordedAt} />
        )}
      </motion.li>
    );
  }

  const isAgent = turn.role === "Agent";
  return (
    <motion.li
      ref={itemRef}
      className={"border-b border-line/70 py-4 last:border-b-0 sm:py-5" + linkedClass(linked)}
      {...enter}
    >
      {turn.entrance && turn.speaker && (
        <LabeledDivider className="mb-4 text-ink-faint">
          <span>{turn.speaker} entered the room</span>
        </LabeledDivider>
      )}
      <div className="mb-1.5 flex items-baseline gap-2">
        <span
          className={
            "font-mono text-2xs font-medium tracking-widest uppercase " +
            (isAgent ? "text-sage" : "text-clay")
          }
        >
          {isAgent ? "the agent" : (turn.speaker ?? "someone")}
        </span>
        {/* A flush turn is internal bookkeeping delivered to no one; mark it as such in faint ink
            rather than the generic "unprompted", so an operator scanning the room sees at a glance
            it was never sent. */}
        {turn.checkpoint ? (
          <span className="font-mono text-2xs text-ink-faint">
            · internal checkpoint · not sent
          </span>
        ) : (
          turn.initiation === "Initiated" &&
          (turn.wakeup ? (
            <span className="font-mono text-2xs text-clay">· woke up · {turn.wakeup}</span>
          ) : (
            <span className="font-mono text-2xs text-ink-faint">· unprompted</span>
          ))
        )}
        {turn.recordedAt > 0 && (
          <span className="ml-auto shrink-0">
            <TurnTimeAnchor roomKey={roomKey} turnId={turn.turnId} recordedAt={turn.recordedAt} />
          </span>
        )}
      </div>
      {/* Deliberation precedes the response — the agent thinks, then speaks. */}
      {(turn.deliberation.length > 0 || inflight) && (
        <Deliberation steps={turn.deliberation} inflight={inflight} />
      )}
      {turn.text ? (
        // Both sides render as Markdown, so a URL is clickable and formatting is honored either way.
        // The agent composes deliberate Markdown; a participant or operator types plain text, so
        // `softBreaks` keeps their single newlines as line breaks.
        <div className={turn.deliberation.length > 0 ? "mt-3" : ""}>
          <TurnMarkdown text={turn.text} softBreaks={!isAgent} />
        </div>
      ) : inflight && !inflight.superseded && inflight.reply ? (
        // The reply streams into the message position it will occupy on commit — same spot, same
        // Markdown rendering, so the committed text simply takes over in place.
        <div className={turn.deliberation.length > 0 || inflight ? "mt-3" : ""}>
          <TurnMarkdown text={inflight.reply} />
        </div>
      ) : inflight ? null : ( // An in-progress turn has no text yet — "silent" is a finished turn's verdict.
        <p
          className={
            "text-sm text-ink-faint italic" + (turn.deliberation.length > 0 ? " mt-3" : "")
          }
        >
          stayed silent
        </p>
      )}
      {turn.outcomes.length > 0 && <Outcomes outcomes={turn.outcomes} />}
      <TurnDebug seq={turn.seq} lastCallSeq={lastCallSeq} tokensOut={tokens.output} />
    </motion.li>
  );
}

/// The turn's debugging surface: a single `debug` disclosure that gathers the final call's context
/// and the underlying `ConversationTurn` event beneath it, so the transcript stays clean until the
/// operator reaches for them. Both nested items disclose through the shared [`Disclosure`], so they
/// carry the same icon and spacing. Rendered only when there is something to show — a final Step call,
/// a landed event, or both.
function TurnDebug({
  seq,
  lastCallSeq,
  tokensOut,
}: {
  seq: number;
  lastCallSeq: number | undefined;
  tokensOut: number | null;
}) {
  const eventsBySeq = useContext(EventsBySeq);
  const [open, setOpen] = useState(false);
  const event = eventsBySeq.get(seq);
  if (lastCallSeq === undefined && !event) return null;
  return (
    <div className="mt-2">
      <Disclosure open={open} onToggle={() => setOpen(!open)} label="debug" />
      {open && (
        <div className="mt-1 flex flex-col gap-1.5 pl-5">
          {event && <TurnEvent event={event} seq={seq} />}
          {lastCallSeq !== undefined && (
            <CallContext seq={lastCallSeq} tokensOut={tokensOut} defaultOpen />
          )}
        </div>
      )}
    </div>
  );
}

/// The turn's underlying `ConversationTurn` event, surfaced by its seq — the coordinate the operator
/// cites when debugging — as a disclosure that expands, in place, into the same viewer the Events tab
/// uses. Sits under the `debug` dropdown alongside the call context, disclosing identically.
function TurnEvent({ event, seq }: { event: Event; seq: number }) {
  const names = useContext(Names);
  const convNames = useContext(ConversationNames);
  const { setSeq } = useStream();
  const [open, setOpen] = useState(true);
  return (
    <div>
      <Disclosure
        open={open}
        onToggle={() => setOpen(!open)}
        label="event"
        summary={`seq ${seq}`}
        onSummaryClick={() => setSeq(seq)}
        summaryTitle="Move the timeline cursor to this event"
      />
      {open && (
        <div className="mt-1 border-l-2 border-line pl-3">
          <EventDetail
            payload={event.payload}
            nameById={names}
            conversationNameById={convNames}
            seq={seq}
            recordedAt={event.recorded_at}
            source={event.source}
          />
        </div>
      )}
    </div>
  );
}

/// A turn's timestamp as its deep-link anchor: a real `<a>` whose href is the Conversation view
/// opened on this room (`…/conversation/<room>`) with the turn pinned (`?turn=<ulid>`), so the
/// browser's own copy-link affordance carries the id the agent's `convo.turn` resolver reads, and a
/// plain click lands on the moment with the arrival wash. The timeline cursor rides along when
/// pinned, so the link reproduces this view of this moment in either frame (live or an eval run).
export function TurnTimeAnchor({
  roomKey,
  turnId,
  recordedAt,
}: {
  roomKey: string;
  turnId: string;
  recordedAt: number;
}) {
  const { seq, link } = useStream();
  return (
    <Link
      to={link.conversation({ room: roomKey, turn: turnId, seq })}
      title={`${formatDateTime(recordedAt)} — a link to this moment; copy the address to cite it`}
      className="font-mono text-2xs text-ink-faint transition-colors hover:text-ink"
    >
      <time dateTime={new Date(recordedAt).toISOString()}>{formatTime(recordedAt)}</time>
    </Link>
  );
}

/// A turn's outcome rows, wired to the cursor's name map from context so each can expand into the
/// event viewer.
function Outcomes({ outcomes }: { outcomes: TurnModel["outcomes"] }) {
  const names = useContext(Names);
  const convNames = useContext(ConversationNames);
  return (
    <OutcomeList
      outcomes={outcomes}
      nameById={names}
      conversationNameById={convNames}
      className="mt-3 gap-1"
    />
  );
}
