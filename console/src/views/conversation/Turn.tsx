import { useContext, useEffect, useRef } from "react";
import { Link, useSearchParams } from "react-router-dom";
import { motion } from "motion/react";

import type { TurnModel } from "../../lib/model/conversation.ts";
import { formatDateTime, formatTime } from "../../lib/format/format.ts";
import { LabeledDivider } from "../../components/primitives.tsx";
import { CallContext } from "./CallContext.tsx";
import { OutcomeList } from "./OutcomeList.tsx";
import { TurnMarkdown } from "./TurnMarkdown.tsx";
import { ConversationNames, ModelCalls, Names } from "./ConversationView.tsx";
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
  const [searchParams] = useSearchParams();
  const linked = searchParams.get("turn") === turn.turnId;
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
            "font-mono text-2xs font-medium uppercase tracking-widest " +
            (isAgent ? "text-sage" : "text-clay")
          }
        >
          {isAgent ? "the agent" : (turn.speaker ?? "someone")}
        </span>
        {turn.initiation === "Initiated" &&
          (turn.wakeup ? (
            <span className="font-mono text-2xs text-clay">· woke up · {turn.wakeup}</span>
          ) : (
            <span className="font-mono text-2xs text-ink-faint">· unprompted</span>
          ))}
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
            "text-sm italic text-ink-faint" + (turn.deliberation.length > 0 ? " mt-3" : "")
          }
        >
          stayed silent
        </p>
      )}
      {turn.outcomes.length > 0 && <Outcomes outcomes={turn.outcomes} />}
      {lastCallSeq !== undefined && (
        <div className="mt-2">
          <CallContext seq={lastCallSeq} tokensOut={tokens.output} />
        </div>
      )}
    </motion.li>
  );
}

/// A turn's timestamp as its deep-link anchor: a real `<a>` whose href is this view's URL with the
/// room and turn pinned (`?room=…&turn=<ulid>`), so the browser's own copy-link affordance carries
/// the id the agent's `convo.turn` resolver reads, and a plain click lands on the moment with the
/// arrival wash. The current params (the timeline cursor included) ride along, so the link
/// reproduces this view of this moment in either frame (live or an eval run).
export function TurnTimeAnchor({
  roomKey,
  turnId,
  recordedAt,
}: {
  roomKey: string;
  turnId: string;
  recordedAt: number;
}) {
  const [searchParams] = useSearchParams();
  const params = new URLSearchParams(searchParams);
  params.set("room", roomKey);
  params.set("turn", turnId);
  return (
    <Link
      to={{ search: params.toString() }}
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
