import { createContext, useContext, useState } from "react";
import { useSearchParams } from "react-router-dom";
import { motion, useReducedMotion } from "motion/react";

import type { Event } from "../types/Event.ts";
import type { Message } from "../types/Message.ts";
import type { Replica } from "../lib/replica.ts";
import type { LiveConnection } from "../lib/live.ts";
import type { ConversationLocator } from "../types/ConversationLocator.ts";
import { completionSummary, nameById, terminalCauseLabel } from "../lib/labels.ts";
import {
  type ConversationModel,
  type DeliberationStep,
  type SessionModel,
  type TurnModel,
  buildConversations,
} from "../lib/conversation.ts";
import { type ModelInteraction, buildInteractions, tokenBudgetAt } from "../lib/interactions.ts";
import { formatDateTime, formatMs, formatTime, formatTokens } from "../lib/format.ts";
import { imprint } from "../lib/operator.ts";
import { DIRECT_PLATFORM, sendMessage } from "../lib/participant.ts";
import { Eyebrow } from "../components/primitives.tsx";
import { Lua } from "../components/Lua.tsx";
import { OutcomeList } from "../components/OutcomeList.tsx";
import { BriefSections } from "../components/BriefTrace.tsx";
import type { BriefTrace } from "../lib/brief.ts";
import { Composer } from "../components/Composer.tsx";
import { Markdown } from "../components/Markdown.tsx";

/// The participate capability the agent frame hands the Conversation view (absent in the eval frame,
/// which is a finished log and so read-only). `atHead` is whether the timeline cursor follows the
/// head — you can speak into the present, but a scrub back is read-only history. `sender` is the
/// handle you converse under as a participant, lifted to the frame so it survives view switches.
export interface Participation {
  connection: LiveConnection;
  sender: string;
  setSender: (value: string) => void;
  atHead: boolean;
}

/// The reconstructed model calls (by their `seq`) and the context budget in effect, so a turn's
/// deliberation can show what each call fed the model and how much of the budget it consumed,
/// without drilling the lookup through every layer of the transcript.
const ModelCalls = createContext<{ bySeq: Map<number, ModelInteraction>; budget: number }>({
  bySeq: new Map(),
  budget: 0,
});

/// The id → handle map at the cursor, so a turn's outcome rows can expand into the event viewer (which
/// resolves memory and participant ids) without drilling the map through the transcript.
const Names = createContext<Map<string, string>>(new Map());

/// The Conversation view: every room the agent speaks in, browsed from a sidebar, with each
/// session's frozen brief and the full transcript — every agent turn openable to the reasoning and
/// Lua behind it, and to the prompt each model call actually saw ("what was the agent thinking,"
/// made literal, spec §Observability). Live, it is also where you *speak*: the console stands in as
/// the agent's `direct` platform client, and the `operator/imprint` room is one entry in the list —
/// selecting it composes with operator authority (the only path that may write `self`). So a single
/// surface watches, replays, and converses.
export function ConversationView({
  replica,
  events,
  cursor,
  participate,
}: {
  replica: Replica;
  events: Event[];
  cursor: number;
  participate?: Participation;
}) {
  const names = nameById(replica.memories(""));
  const conversations = buildConversations(
    events.filter((event) => event.seq <= cursor),
    names,
  );
  const modelCalls = {
    bySeq: new Map(buildInteractions(events, cursor).map((call) => [call.seq, call])),
    budget: tokenBudgetAt(events, cursor),
  };
  // The open room rides in the URL (`?room`), so it deep-links, survives a view switch, and moves
  // with browser back and forward like the rest of the stream's state.
  const [searchParams, setSearchParams] = useSearchParams();
  const selectedKey = searchParams.get("room");
  const [draftRoom, setDraftRoom] = useState("");
  // A room the operator named but has not sent to yet — held as its own locator rather than packed
  // into a key, so it survives until its first message creates it on the log.
  const [pendingRoom, setPendingRoom] = useState<ConversationLocator | null>(null);

  // Participant rooms (most recent first) and the operator/imprint room, kept apart so the latter
  // pins to the bottom and is marked — live, it is offered even before its first message.
  const participants = conversations
    .filter((conversation) => conversation.platform !== "operator")
    .map(toChannel)
    .sort((a, b) => lastActivity(b.conversation) - lastActivity(a.conversation));
  const operator = conversations.find((conversation) => conversation.platform === "operator");
  const operatorChannel: Channel | null = operator
    ? toChannel(operator)
    : participate
      ? imprintChannel()
      : null;

  // The pending room shows until a real conversation of the same locator appears on the log.
  const known = [...participants, operatorChannel].filter((channel) => channel !== null);
  const pending =
    pendingRoom && !known.some((channel) => channel.key === localKey(pendingRoom))
      ? newRoomChannel(pendingRoom)
      : null;
  const listed = pending ? [pending, ...participants] : participants;
  const selected =
    known.find((channel) => channel.key === selectedKey) ??
    pending ??
    participants[0] ??
    operatorChannel ??
    null;

  function selectRoom(key: string) {
    setSearchParams(
      (prev) => {
        const updated = new URLSearchParams(prev);
        updated.set("room", key);
        return updated;
      },
      { replace: true },
    );
  }

  function startRoom() {
    const name = draftRoom.trim();
    if (!name) return;
    const locator = { platform: DIRECT_PLATFORM, scope_path: name };
    setPendingRoom(locator);
    selectRoom(localKey(locator));
    setDraftRoom("");
  }

  return (
    <ModelCalls.Provider value={modelCalls}>
      <Names.Provider value={names}>
        <div className="grid grid-cols-1 gap-5 md:grid-cols-[11rem_1fr] md:gap-6">
          <div className="md:sticky md:top-4 md:self-start">
            <aside className="hidden flex-col gap-5 md:flex">
              {participate && (
                <div className="flex items-baseline gap-2 font-mono text-2xs text-ink-faint">
                  <span className="text-line-strong">+</span>
                  <input
                    value={draftRoom}
                    onChange={(event) => setDraftRoom(event.target.value)}
                    onKeyDown={(event) => event.key === "Enter" && startRoom()}
                    placeholder="new conversation"
                    className="flex-1 bg-transparent placeholder:text-ink-faint/60 focus:outline-none"
                  />
                </div>
              )}

              {listed.length === 0 && !operatorChannel ? (
                <p className="font-mono text-2xs text-ink-faint">no conversations yet</p>
              ) : (
                <nav className="flex flex-col gap-1">
                  {listed.map((channel) => (
                    <ChannelLink
                      key={channel.key}
                      channel={channel}
                      active={channel.key === selected?.key}
                      onSelect={() => selectRoom(channel.key)}
                    />
                  ))}
                </nav>
              )}

              {operatorChannel && (
                <div className="border-t border-line pt-4">
                  <Eyebrow>operator</Eyebrow>
                  <nav className="mt-2">
                    <ChannelLink
                      channel={operatorChannel}
                      active={operatorChannel.key === selected?.key}
                      onSelect={() => selectRoom(operatorChannel.key)}
                    />
                  </nav>
                </div>
              )}

              {participate && (
                <label className="mt-2 flex flex-col gap-1.5 border-t border-line pt-4">
                  <Eyebrow>you are</Eyebrow>
                  <input
                    value={participate.sender}
                    onChange={(event) => participate.setSender(event.target.value)}
                    placeholder="a handle"
                    className="w-full border-b border-line bg-transparent pb-1 font-mono text-xs text-ink placeholder:text-ink-faint/60 focus:border-ink-faint focus:outline-none"
                  />
                </label>
              )}
            </aside>

            {/* On mobile the list collapses to a dropdown so the transcript owns the screen. */}
            <div className="flex flex-col gap-3 md:hidden">
              <ChannelSelect
                listed={listed}
                operatorChannel={operatorChannel}
                selectedKey={selected?.key ?? null}
                onSelect={selectRoom}
              />
              {participate && (
                <div className="flex flex-wrap items-baseline gap-x-5 gap-y-2 font-mono text-2xs text-ink-faint">
                  <span className="flex items-baseline gap-2">
                    <span className="text-line-strong">+</span>
                    <input
                      value={draftRoom}
                      onChange={(event) => setDraftRoom(event.target.value)}
                      onKeyDown={(event) => event.key === "Enter" && startRoom()}
                      placeholder="new conversation"
                      className="w-36 bg-transparent placeholder:text-ink-faint/60 focus:outline-none"
                    />
                  </span>
                  <span className="flex items-baseline gap-2">
                    <Eyebrow>you are</Eyebrow>
                    <input
                      value={participate.sender}
                      onChange={(event) => participate.setSender(event.target.value)}
                      placeholder="a handle"
                      className="w-24 bg-transparent text-ink placeholder:text-ink-faint/60 focus:outline-none"
                    />
                  </span>
                </div>
              )}
            </div>
          </div>

          {selected ? (
            // Keyed by room, so per-room composer state (the in-flight optimistic turn) resets on a
            // channel switch rather than leaking the last room's pending message into the next.
            <Room
              key={selected.key}
              replica={replica}
              cursor={cursor}
              channel={selected}
              participate={participate}
            />
          ) : (
            <div className="py-16 text-center text-sm text-ink-faint">
              {participate ? "Name a conversation to start one." : "No conversations in this run."}
            </div>
          )}
        </div>
      </Names.Provider>
    </ModelCalls.Provider>
  );
}

/// One conversation, open: its header, sessions, and transcript, plus — live and at the head — a
/// composer routed to the room's authority (the imprint room writes `self`; the rest are ordinary
/// participant turns).
function Room({
  replica,
  cursor,
  channel,
  participate,
}: {
  replica: Replica;
  cursor: number;
  channel: Channel;
  participate?: Participation;
}) {
  const isOperator = channel.authority === "operator";
  const handle = participate?.sender.trim() ?? "";
  const { bySeq, budget } = useContext(ModelCalls);
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
    budget > 0 ? Math.round((convoTokens.peakContext / budget) * 100) : null;
  // True while a sent turn is in flight, so the conversation shows the agent at work.
  const [thinking, setThinking] = useState(false);
  // The just-sent turn, shown optimistically until the live tail folds the real one in — so the
  // message appears the instant it is sent rather than after the round-trip, and the thinking pulse
  // never sits against a conversation that does not yet show what was said. `baseline` is the turn
  // count at send; once the conversation grows past it, the real turn has landed and this is dropped.
  const [optimistic, setOptimistic] = useState<{ text: string; baseline: number } | null>(null);

  async function onSend(text: string) {
    if (!participate) return;
    setOptimistic({ text, baseline: channel.conversation?.turns.length ?? 0 });
    try {
      if (isOperator) {
        await imprint(participate.connection, text);
      } else {
        await sendMessage(participate.connection, {
          locator: channel.locator,
          sender: handle,
          text,
          present: [handle],
        });
      }
    } catch (error) {
      setOptimistic(null); // the send failed — drop the optimistic turn (the composer restores the draft).
      throw error;
    }
  }

  return (
    <div className="flex w-full max-w-prose flex-col">
      <header className="mb-5 sm:mb-6">
        <h2 className="font-serif text-xl text-ink sm:text-2xl">{channel.label}</h2>
        {/* The locator addresses a real room; for the operator channel it just echoes the title. */}
        {!isOperator && (
          <p className="mt-1 font-mono text-2xs uppercase tracking-widest text-ink-faint">
            {channel.locator.platform} · {channel.locator.scope_path}
          </p>
        )}
        {convoTokens.peakContext + convoTokens.output > 0 && (
          <p className="mt-1 font-mono text-2xs text-ink-faint">
            {formatTokens(convoTokens.output)} generated · peak{" "}
            {formatTokens(convoTokens.peakContext)}
            {convoTowardCompaction !== null && <> · {convoTowardCompaction}% to compaction</>}
          </p>
        )}
      </header>

      {channel.conversation ? (
        <Transcript replica={replica} conversation={channel.conversation} cursor={cursor} />
      ) : (
        <p className="py-7 text-sm text-ink-faint">
          {isOperator
            ? "Introduce yourself to begin the interview."
            : "No messages yet — say hello."}
        </p>
      )}

      {optimistic !== null && (channel.conversation?.turns.length ?? 0) <= optimistic.baseline && (
        <OptimisticTurn speaker={handle || "you"} text={optimistic.text} />
      )}

      {thinking && <ThinkingIndicator />}

      {participate &&
        (participate.atHead ? (
          <div className="mt-6">
            <Composer
              onSend={onSend}
              onPendingChange={setThinking}
              disabled={!isOperator && handle.length === 0}
              disabledHint="Set who you are to start."
              placeholder={
                isOperator
                  ? "Speak to the agent as the operator…"
                  : `Message ${channel.label} as ${handle || "…"}`
              }
            />
          </div>
        ) : (
          <p className="mt-6 border-t border-line pt-4 text-center font-mono text-2xs text-ink-faint">
            viewing history · return to the head of the timeline to speak
          </p>
        ))}
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
        <span className="font-mono text-2xs uppercase tracking-widest text-clay">{speaker}</span>
        <span className="ml-auto shrink-0 font-mono text-2xs text-ink-faint">sending…</span>
      </div>
      <p className="whitespace-pre-wrap text-base leading-relaxed text-ink">{text}</p>
    </div>
  );
}

function Transcript({
  replica,
  conversation,
  cursor,
}: {
  replica: Replica;
  conversation: ConversationModel;
  cursor: number;
}) {
  // Turns already present when this conversation first rendered are the "initial state" and sit still;
  // turns that arrive afterward — a live run streaming in — fade and slide in to signal the new state.
  const reduce = useReducedMotion();
  const [freshAfter] = useState(cursor);
  if (conversation.sessions.length === 0) {
    return (
      <ol className="flex flex-col">
        {conversation.turns.map((turn) => (
          <TurnItem key={turn.turnId} turn={turn} fresh={!reduce && turn.seq > freshAfter} />
        ))}
      </ol>
    );
  }
  return (
    <>
      {conversation.sessions.map((session, index) => {
        // Each session owns the turns from its open until the next session re-segments.
        const fromSeq = index === 0 ? 0 : session.seq;
        const toSeq = conversation.sessions[index + 1]?.seq ?? Infinity;
        const turns = conversation.turns.filter((turn) => turn.seq >= fromSeq && turn.seq < toSeq);
        return (
          <div key={session.id}>
            <SessionDivider session={session} first={index === 0} />
            <BriefBlock
              replica={replica}
              session={session}
              contextMemory={conversation.contextMemory}
            />
            <ol className="mt-2 flex flex-col">
              {turns.map((turn) => (
                <TurnItem key={turn.turnId} turn={turn} fresh={!reduce && turn.seq > freshAfter} />
              ))}
            </ol>
          </div>
        );
      })}
    </>
  );
}

/// A conversation as the sidebar and composer see it: a stable key (its locator, so a pending room
/// and the real one it becomes share an entry), a label, the locator to address it, the authority
/// composing into it carries, and the folded model (`null` until it exists on the log).
interface Channel {
  key: string;
  label: string;
  locator: ConversationLocator;
  authority: "operator" | "participant";
  conversation: ConversationModel | null;
}

/// The mobile face of the conversation list: a native dropdown (participant rooms, then the operator
/// room as its own group) so the transcript owns the screen. Hidden once the sidebar fits (`md`).
function ChannelSelect({
  listed,
  operatorChannel,
  selectedKey,
  onSelect,
}: {
  listed: Channel[];
  operatorChannel: Channel | null;
  selectedKey: string | null;
  onSelect: (key: string) => void;
}) {
  return (
    <select
      value={selectedKey ?? ""}
      onChange={(event) => onSelect(event.target.value)}
      className="w-full border border-line bg-paper px-3 py-2 text-sm text-ink focus:border-ink-faint focus:outline-none"
      aria-label="Choose a conversation"
    >
      {listed.length > 0 && (
        <optgroup label="conversations">
          {listed.map((channel) => (
            <option key={channel.key} value={channel.key}>
              {channel.label}
            </option>
          ))}
        </optgroup>
      )}
      {operatorChannel && (
        <optgroup label="operator">
          <option value={operatorChannel.key}>{operatorChannel.label}</option>
        </optgroup>
      )}
    </select>
  );
}

function ChannelLink({
  channel,
  active,
  onSelect,
}: {
  channel: Channel;
  active: boolean;
  onSelect: () => void;
}) {
  return (
    <button
      onClick={onSelect}
      title={channel.label}
      className={
        "-ml-3 flex w-full min-w-0 items-baseline border-l-2 py-1 pl-2.5 text-left text-sm transition-colors " +
        (active ? "border-clay text-ink" : "border-transparent text-ink-soft hover:text-ink") +
        (channel.conversation ? "" : " italic text-ink-faint")
      }
    >
      <span className="truncate">{channel.label}</span>
    </button>
  );
}

function toChannel(conversation: ConversationModel): Channel {
  return {
    key: channelKey(conversation.platform, conversation.scopePath),
    label: conversation.contextName ?? `${conversation.platform}:${conversation.scopePath}`,
    locator: { platform: conversation.platform, scope_path: conversation.scopePath },
    authority: conversation.platform === "operator" ? "operator" : "participant",
    conversation,
  };
}

function imprintChannel(): Channel {
  return {
    key: channelKey("operator", "imprint"),
    label: "imprint",
    locator: { platform: "operator", scope_path: "imprint" },
    authority: "operator",
    conversation: null,
  };
}

function newRoomChannel(locator: ConversationLocator): Channel {
  return {
    key: localKey(locator),
    label: locator.scope_path,
    locator,
    authority: "participant",
    conversation: null,
  };
}

function channelKey(platform: string, scopePath: string): string {
  return localKey({ platform, scope_path: scopePath });
}

/// A stable selection key for a room — its locator, never parsed back, so the separator is only a
/// display concern and a room name may contain anything.
function localKey(locator: ConversationLocator): string {
  return `${locator.platform} · ${locator.scope_path}`;
}

function lastActivity(conversation: ConversationModel | null): number {
  return conversation ? conversation.turns.reduce((max, turn) => Math.max(max, turn.seq), 0) : 0;
}

/// The seam between conversations in one context. A context can hold several conversations over time
/// — each opens its own session with a freshly composed brief — and previously the only sign of a new
/// one was the brief reappearing. This draws the boundary plainly: a labelled rule with the date the
/// conversation opened, reading "conversation" at the context's first, "new conversation" at each one
/// after, and "re-briefed · compaction" when a session reopened by re-segmenting the last rather than
/// starting fresh.
function SessionDivider({ session, first }: { session: SessionModel; first: boolean }) {
  const label = session.compaction
    ? "re-briefed · compaction"
    : first
      ? "conversation"
      : "new conversation";
  return (
    <div
      className={
        "flex items-center gap-3 " +
        (first ? "mb-4 " : "mb-4 mt-7 ") +
        (session.compaction ? "text-clay" : "text-ink-soft")
      }
    >
      <span className="h-px flex-1 bg-line" />
      <span className="flex items-baseline gap-2 font-mono text-2xs uppercase tracking-widest">
        <span>{label}</span>
        <span className="tracking-normal text-ink-faint normal-case">
          {formatDateTime(session.startedAt)}
        </span>
      </span>
      <span className="h-px flex-1 bg-line" />
    </div>
  );
}

/// The brief the agent saw, frozen at the session's open: the literal text (`session.brief`, captured
/// on `SessionStarted`) directly, and — one level deeper, behind its own toggle — the composer's trace
/// (which memories it weighed, and why each entry was surfaced, trimmed, or filtered). The trace is
/// gated because evaluating it re-folds the replica to the session's seq, so it reflects the frozen
/// point rather than the cursor; that re-fold is paid only when asked for, and cached once.
function BriefBlock({
  replica,
  session,
  contextMemory,
}: {
  replica: Replica;
  session: SessionModel;
  contextMemory: string | null;
}) {
  const [open, setOpen] = useState(false);
  const [traceOpen, setTraceOpen] = useState(false);
  const [trace, setTrace] = useState<BriefTrace | null>(null);

  function toggleTrace() {
    // Compose the trace at the session's open seq — re-fold there, read, restore the fold, all
    // synchronously in this handler so the rest of the view never observes the moved fold.
    if (trace === null) {
      const restore = replica.foldedSeq;
      replica.foldTo(session.seq);
      setTrace(replica.brief(session.participantIds, contextMemory, session.startedAt));
      replica.foldTo(restore);
    }
    setTraceOpen(!traceOpen);
  }

  return (
    <div className="mb-6 border-b border-line pb-6">
      <button
        onClick={() => setOpen(!open)}
        className="flex items-baseline gap-3 text-left transition-colors hover:text-ink"
      >
        <Eyebrow>{open ? "▾ brief" : "▸ brief"}</Eyebrow>
        <span className="font-mono text-2xs text-ink-faint">
          {session.participants.join(", ") || "no one present"}
        </span>
      </button>
      {open && (
        <>
          <pre className="mt-3 max-h-96 overflow-auto whitespace-pre-wrap border-l border-line bg-oat/40 px-4 py-3 font-mono text-2xs leading-relaxed text-ink-soft">
            {session.brief}
          </pre>
          <button
            onClick={toggleTrace}
            className="mt-3 font-mono text-2xs text-ink-faint transition-colors hover:text-ink-soft"
          >
            {traceOpen ? "▾" : "▸"} composition trace
            <span className="ml-2 text-ink-faint/70">· re-folds the replica to evaluate</span>
          </button>
          {traceOpen && trace && <BriefSections sections={trace.sections} />}
        </>
      )}
    </div>
  );
}

/// A turn's measured token cost. `context` is the *peak* prompt across its model calls — the largest
/// context the model read this turn. It is cumulative by nature (each step re-sends the whole growing
/// buffer, which itself carries every prior turn), and it is exactly what the compaction trigger
/// weighs against the budget (server/platform.rs: a turn compacts when its peak prompt crosses
/// `token_budget`). `output` is the *sum* of completions — the tokens the agent generated, which is
/// additive with no overlap. Both are 0 for a participant or system turn (no model call): a
/// participant message's own tokens are not measured, only folded into the next agent prompt.
function turnTokens(
  turn: TurnModel,
  bySeq: Map<number, ModelInteraction>,
): { context: number; output: number } {
  let context = 0;
  let output = 0;
  for (const step of turn.deliberation) {
    if (step.kind !== "model") continue;
    const usage = bySeq.get(step.seq)?.usage;
    context = Math.max(context, usage?.prompt_tokens ?? 0);
    output += usage?.completion_tokens ?? 0;
  }
  return { context, output };
}

function TurnItem({ turn, fresh }: { turn: TurnModel; fresh: boolean }) {
  const { bySeq, budget } = useContext(ModelCalls);
  const tokens = turnTokens(turn, bySeq);
  const towardCompaction = budget > 0 ? Math.round((tokens.context / budget) * 100) : null;
  // A turn that streamed in after the view opened fades and lifts into place; the initial ones do not.
  const enter = fresh
    ? {
        initial: { opacity: 0, y: 6 },
        animate: { opacity: 1, y: 0 },
        transition: { duration: 0.35, ease: [0.32, 0.72, 0, 1] as const },
      }
    : {};
  if (turn.role === "System") {
    return (
      <motion.li className="py-3 text-center" {...enter}>
        <span className="font-mono text-2xs text-ink-faint">{turn.text || "(system)"}</span>
      </motion.li>
    );
  }

  const isAgent = turn.role === "Agent";
  return (
    <motion.li className="border-b border-line/70 py-4 last:border-b-0 sm:py-5" {...enter}>
      {turn.entrance && turn.speaker && (
        <div className="mb-4 flex items-center gap-3 text-ink-faint">
          <span className="h-px flex-1 bg-line" />
          <span className="font-mono text-2xs">{turn.speaker} entered the room</span>
          <span className="h-px flex-1 bg-line" />
        </div>
      )}
      <div className="mb-1.5 flex items-baseline gap-2">
        <span
          className={
            "font-mono text-2xs uppercase tracking-widest " + (isAgent ? "text-sage" : "text-clay")
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
          <time
            className="ml-auto shrink-0 font-mono text-2xs text-ink-faint"
            dateTime={new Date(turn.recordedAt).toISOString()}
            title={formatDateTime(turn.recordedAt)}
          >
            {formatTime(turn.recordedAt)}
          </time>
        )}
      </div>
      {/* Deliberation precedes the response — the agent thinks, then speaks. */}
      {turn.deliberation.length > 0 && <Deliberation steps={turn.deliberation} />}
      {turn.text ? (
        isAgent ? (
          // The agent composes its replies as Markdown; render them so. Participant and operator input
          // stays raw text below — only its line breaks are preserved.
          <div className={turn.deliberation.length > 0 ? "mt-3" : ""}>
            <Markdown text={turn.text} />
          </div>
        ) : (
          <p
            className={
              "whitespace-pre-wrap text-base leading-relaxed text-ink" +
              (turn.deliberation.length > 0 ? " mt-3" : "")
            }
          >
            {turn.text}
          </p>
        )
      ) : (
        <p
          className={
            "text-sm italic text-ink-faint" + (turn.deliberation.length > 0 ? " mt-3" : "")
          }
        >
          stayed silent
        </p>
      )}
      {turn.outcomes.length > 0 && <Outcomes outcomes={turn.outcomes} />}
      {/* The agent turn's cost, footing the turn: the context it read (cumulative — the whole re-sent
          buffer) as a fill against the compaction budget, and the tokens it generated (additive). */}
      {tokens.output + tokens.context > 0 && (
        <div className="mt-3 flex items-center gap-2 font-mono text-2xs text-ink-faint">
          <span>
            {formatTokens(tokens.context)} cumulative input · {formatTokens(tokens.output)} out
            {towardCompaction !== null && " ·"}
          </span>
          {towardCompaction !== null && (
            <>
              <div
                className="h-1 w-16 shrink-0 bg-oat"
                title={`${towardCompaction}% of the compaction budget (${formatTokens(budget)})`}
              >
                <div
                  className={"h-1 " + (towardCompaction >= 80 ? "bg-clay" : "bg-sage")}
                  style={{ width: `${Math.min(100, towardCompaction)}%` }}
                />
              </div>
              <span>
                {towardCompaction}% to compaction ({formatTokens(budget)})
              </span>
            </>
          )}
        </div>
      )}
    </motion.li>
  );
}

/// A turn's outcome rows, wired to the cursor's name map from context so each can expand into the
/// event viewer.
function Outcomes({ outcomes }: { outcomes: TurnModel["outcomes"] }) {
  const names = useContext(Names);
  return <OutcomeList outcomes={outcomes} nameById={names} className="mt-3 gap-1" />;
}

function Deliberation({ steps }: { steps: DeliberationStep[] }) {
  const [open, setOpen] = useState(false);
  const total = steps.reduce((sum, step) => sum + step.durationMs, 0);

  return (
    <div className="mt-3">
      <button
        onClick={() => setOpen(!open)}
        className="font-mono text-2xs text-ink-faint transition-colors hover:text-ink-soft"
      >
        {open ? "▾" : "▸"} deliberation · {steps.length} step{steps.length > 1 ? "s" : ""} ·{" "}
        {formatMs(total)}
      </button>
      {open && (
        <div className="mt-3 flex flex-col gap-3 border-l border-line pl-4">
          {steps.map((step, index) =>
            step.kind === "model" ? (
              <ModelStep key={index} step={step} />
            ) : (
              <LuaStep key={index} step={step} />
            ),
          )}
        </div>
      )}
    </div>
  );
}

function ModelStep({ step }: { step: Extract<DeliberationStep, { kind: "model" }> }) {
  const { bySeq, budget } = useContext(ModelCalls);
  const interaction = bySeq.get(step.seq);
  const [showPrompt, setShowPrompt] = useState(false);
  return (
    <div>
      <div className="flex items-baseline gap-2 font-mono text-2xs text-ink-faint">
        <span className="lowercase">{step.phase}</span>
        <span className="text-ink-faint/45">·</span>
        <span>{completionSummary(step.completion)}</span>
        <span className="text-ink-faint/45">·</span>
        <span>{formatMs(step.durationMs)}</span>
      </div>
      {interaction && <ContextBar usage={interaction.usage} budget={budget} />}
      {step.reasoning && (
        <p className="mt-1 font-serif text-sm italic leading-relaxed text-ink-soft">
          {step.reasoning}
        </p>
      )}
      {interaction && (interaction.system || interaction.messages.length > 0) && (
        <div className="mt-1.5">
          <button
            onClick={() => setShowPrompt(!showPrompt)}
            className="font-mono text-2xs text-ink-faint transition-colors hover:text-ink-soft"
          >
            {showPrompt ? "▾" : "▸"} prompt · {interaction.messages.length} message
            {interaction.messages.length === 1 ? "" : "s"}
          </button>
          {showPrompt && <Prompt interaction={interaction} />}
        </div>
      )}
    </div>
  );
}

/// How much of the context budget this call's prompt consumed — a slim bar filling toward the budget,
/// sage with headroom and clay as it nears it (where compaction looms), with the token counts.
function ContextBar({ usage, budget }: { usage: ModelInteraction["usage"]; budget: number }) {
  if (usage.prompt_tokens === null) return null;
  const fraction = budget > 0 ? usage.prompt_tokens / budget : 0;
  return (
    <div className="mt-1.5 flex items-center gap-3">
      <div className="h-1 w-24 shrink-0 bg-oat">
        <div
          className={"h-1 " + (fraction >= 0.8 ? "bg-clay" : "bg-sage")}
          style={{ width: `${Math.min(100, fraction * 100)}%` }}
        />
      </div>
      <span className="font-mono text-2xs text-ink-faint">
        {formatTokens(usage.prompt_tokens)} / {formatTokens(budget)} · {Math.round(fraction * 100)}%
        {usage.completion_tokens !== null && ` · +${formatTokens(usage.completion_tokens)} out`}
      </span>
    </div>
  );
}

/// The full prompt the model saw, reconstructed from the delta-encoded request: the system prompt,
/// the messages, and the tools offered.
function Prompt({ interaction }: { interaction: ModelInteraction }) {
  return (
    <div className="mt-2 flex flex-col gap-3 border-l border-line pl-4">
      <div>
        <Eyebrow>system</Eyebrow>
        <Block text={interaction.system || "(none)"} />
      </div>
      <div>
        <Eyebrow>messages</Eyebrow>
        <div className="mt-1 flex flex-col gap-2">
          {interaction.messages.map((message, index) => (
            <MessageRow key={index} message={message} />
          ))}
        </div>
      </div>
      {interaction.tools.length > 0 && (
        <p className="font-mono text-2xs text-ink-faint">
          tools · {interaction.tools.map((tool) => tool.name).join(", ")}
        </p>
      )}
    </div>
  );
}

function MessageRow({ message }: { message: Message }) {
  return (
    <div>
      <span className="font-mono text-2xs uppercase tracking-widest text-ink-faint">
        {message.role}
      </span>
      {message.content && <Block text={message.content} />}
      {message.tool_calls.length > 0 && (
        <p className="mt-1 font-mono text-2xs text-clay">
          → {message.tool_calls.map((call) => call.name).join(", ")}
        </p>
      )}
    </div>
  );
}

function Block({ text }: { text: string }) {
  return (
    <pre className="mt-1 max-h-72 overflow-auto whitespace-pre-wrap border-l border-line bg-oat/40 px-3 py-2 font-mono text-2xs leading-relaxed text-ink-soft">
      {text}
    </pre>
  );
}

function LuaStep({ step }: { step: Extract<DeliberationStep, { kind: "lua" }> }) {
  const error = step.terminalCause;
  return (
    <div>
      <Lua code={step.script} />
      {error ? (
        <p className="mt-1 font-mono text-2xs text-clay">{terminalCauseLabel(error)}</p>
      ) : (
        step.result && (
          <p className="mt-1 whitespace-pre-wrap font-mono text-2xs text-ink-soft">
            → {step.result}
          </p>
        )
      )}
    </div>
  );
}
