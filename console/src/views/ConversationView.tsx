import { createContext, useContext, useState } from "react";

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
import { formatMs, formatTokens } from "../lib/format.ts";
import { imprint } from "../lib/operator.ts";
import { DIRECT_PLATFORM, sendMessage } from "../lib/participant.ts";
import { Eyebrow } from "../components/primitives.tsx";
import { Lua } from "../components/Lua.tsx";
import { OutcomeList } from "../components/OutcomeList.tsx";
import { BriefTraceView } from "../components/BriefTrace.tsx";
import { Composer } from "../components/Composer.tsx";

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
  const conversations = buildConversations(
    events.filter((event) => event.seq <= cursor),
    nameById(replica.memories("")),
  );
  const modelCalls = {
    bySeq: new Map(buildInteractions(events, cursor).map((call) => [call.seq, call])),
    budget: tokenBudgetAt(events, cursor),
  };
  const [selectedKey, setSelectedKey] = useState<string | null>(null);
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

  function startRoom() {
    const name = draftRoom.trim();
    if (!name) return;
    const locator = { platform: DIRECT_PLATFORM, scope_path: name };
    setPendingRoom(locator);
    setSelectedKey(localKey(locator));
    setDraftRoom("");
  }

  return (
    <ModelCalls.Provider value={modelCalls}>
      <div className="grid grid-cols-1 gap-5 md:grid-cols-[11rem_1fr] md:gap-8">
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
                    onSelect={() => setSelectedKey(channel.key)}
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
                    onSelect={() => setSelectedKey(operatorChannel.key)}
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
              onSelect={setSelectedKey}
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
          <Room replica={replica} cursor={cursor} channel={selected} participate={participate} />
        ) : (
          <div className="py-24 text-center text-sm text-ink-faint">
            {participate ? "Name a conversation to start one." : "No conversations in this run."}
          </div>
        )}
      </div>
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

  async function onSend(text: string) {
    if (!participate) return;
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
  }

  return (
    <div className="flex w-full max-w-prose flex-col">
      <header className="mb-5 sm:mb-8">
        <div className="flex flex-wrap items-baseline gap-x-3 gap-y-1">
          <h2 className="font-serif text-xl text-ink sm:text-2xl">{channel.label}</h2>
          {isOperator && <Eyebrow>operator authority · writes self</Eyebrow>}
        </div>
        <p className="mt-1 font-mono text-2xs uppercase tracking-widest text-ink-faint">
          {channel.locator.platform} · {channel.locator.scope_path}
        </p>
      </header>

      {channel.conversation ? (
        <Transcript replica={replica} conversation={channel.conversation} cursor={cursor} />
      ) : (
        <p className="py-10 text-sm text-ink-faint">
          {isOperator
            ? "Introduce yourself to begin the interview."
            : "No messages yet — say hello."}
        </p>
      )}

      {participate &&
        (participate.atHead ? (
          <div className="mt-6">
            <Composer
              onSend={onSend}
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

function Transcript({
  replica,
  conversation,
  cursor,
}: {
  replica: Replica;
  conversation: ConversationModel;
  cursor: number;
}) {
  if (conversation.sessions.length === 0) {
    return (
      <ol className="flex flex-col">
        {conversation.turns.map((turn) => (
          <TurnItem key={turn.turnId} turn={turn} />
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
            {session.compaction && (
              <div className="my-7 flex items-center gap-3 text-clay">
                <span className="h-px flex-1 bg-line" />
                <span className="font-mono text-2xs uppercase tracking-widest">
                  re-briefed · compaction
                </span>
                <span className="h-px flex-1 bg-line" />
              </div>
            )}
            <BriefBlock
              replica={replica}
              session={session}
              contextMemory={conversation.contextMemory}
              cursor={cursor}
            />
            <ol className="mt-2 flex flex-col">
              {turns.map((turn) => (
                <TurnItem key={turn.turnId} turn={turn} />
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

function BriefBlock({
  replica,
  session,
  contextMemory,
  cursor,
}: {
  replica: Replica;
  session: SessionModel;
  contextMemory: string | null;
  cursor: number;
}) {
  const [open, setOpen] = useState(false);
  return (
    <div className="mb-6 border-b border-line pb-6">
      <button
        onClick={() => setOpen(!open)}
        className="flex items-baseline gap-3 text-left transition-colors hover:text-ink"
      >
        <Eyebrow>{open ? "▾ brief" : "▸ brief"}</Eyebrow>
        <span className="font-mono text-2xs text-ink-faint">
          how it was composed · {session.participants.join(", ") || "no one present"}
        </span>
      </button>
      {open && (
        <BriefComposition
          key={cursor}
          replica={replica}
          session={session}
          contextMemory={contextMemory}
        />
      )}
    </div>
  );
}

/// Re-derives the brief at the current timeline cursor (hence keyed by it in the parent, so a scrub
/// re-runs the composer) and renders its trace.
function BriefComposition({
  replica,
  session,
  contextMemory,
}: {
  replica: Replica;
  session: SessionModel;
  contextMemory: string | null;
}) {
  const trace = replica.brief(session.participantIds, contextMemory, session.startedAt);
  return <BriefTraceView trace={trace} />;
}

function TurnItem({ turn }: { turn: TurnModel }) {
  if (turn.role === "System") {
    return (
      <li className="py-3 text-center">
        <span className="font-mono text-2xs text-ink-faint">{turn.text || "(system)"}</span>
      </li>
    );
  }

  const isAgent = turn.role === "Agent";
  return (
    <li className="border-b border-line/70 py-4 last:border-b-0 sm:py-5">
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
      </div>
      {turn.text ? (
        <p className="text-base leading-relaxed text-ink">{turn.text}</p>
      ) : (
        <p className="text-sm italic text-ink-faint">stayed silent</p>
      )}
      {turn.deliberation.length > 0 && <Deliberation steps={turn.deliberation} />}
      {turn.outcomes.length > 0 && <OutcomeList outcomes={turn.outcomes} className="mt-3 gap-1" />}
    </li>
  );
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
