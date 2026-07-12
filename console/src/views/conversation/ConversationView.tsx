import { createContext, useState } from "react";
import { useSearchParams } from "react-router-dom";

import type { Event } from "../../types/Event.ts";
import { type DigestStatus, type Replica } from "../../lib/replica/replica.ts";
import { nameById } from "../../lib/model/labels.ts";
import type { LiveConnection } from "../../lib/api/live.ts";
import type { InFlightGeneration } from "../../lib/model/inflight.ts";
import type { ConversationLocator } from "../../types/ConversationLocator.ts";
import { buildConversations } from "../../lib/model/conversation.ts";
import { conversationNameById } from "../../components/EventDetail.tsx";
import { type ContextDebug, deriveContextDebug } from "../../lib/model/contextDebug.ts";
import { DIRECT_PLATFORM } from "../../lib/api/participant.ts";
import { Eyebrow } from "../../components/primitives.tsx";
import { type TurnRefTarget, TurnRefs } from "../../lib/view/turnRefs.ts";
import {
  type Channel,
  channelKey,
  groupChannels,
  hasScopeChar,
  imprintChannel,
  localKey,
  newRoomChannel,
  toChannel,
} from "./channelUtilities.tsx";
import { ChannelLink, ChannelSelect, MobileRoomControls, RoomControls } from "./Channels.tsx";
import { Room } from "./Room.tsx";

/// The participate capability the agent frame hands the Conversation view (absent in the eval frame,
/// which is a finished log and so read-only). `sender` is the handle you converse under as a
/// participant, lifted to the frame so it survives view switches. Whether the cursor is at the head
/// — the gate on speaking into the present, and on following the live tail — rides the view's own
/// `atHead` prop, since a read-only eval run at its head follows the tail too.
export interface Participation {
  connection: LiveConnection;
  sender: string;
  setSender: (value: string) => void;
}

/// The reconstructed model calls with their derived context debugging — per-call cache verdicts,
/// token attributions, digest verifications, and the denominators in effect — so a turn's
/// deliberation can show what each call fed the model, how it was assembled, and whether the prefix
/// cache survived, without drilling the lookups through every layer of the transcript.
export const ModelCalls = createContext<ContextDebug & { digestBySeq: Map<number, DigestStatus> }>({
  bySeq: new Map(),
  verdictBySeq: new Map(),
  attributionBySeq: new Map(),
  denominatorsBySeq: new Map(),
  denominators: { budget: null, contextLength: null },
  digestBySeq: new Map(),
});

/// The id → handle map at the cursor, so a turn's outcome rows can expand into the event viewer (which
/// resolves memory and participant ids) without drilling the map through the transcript.
export const Names = createContext<Map<string, string>>(new Map());

/// The conversation id → context memory name map at the cursor, so `ConversationRef` links in
/// event detail panels can resolve the room name without a separate prop chain.
export const ConversationNames = createContext<Map<string, string>>(new Map());

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
  atHead = false,
  participate,
  progress,
}: {
  replica: Replica;
  events: Event[];
  cursor: number;
  /// Whether the timeline cursor sits at the head. Gates speaking into the present (live) and
  /// following the live tail as new turns and tokens arrive (both frames), so a run watched at its
  /// head auto-scrolls while a scrub back into history is left undisturbed.
  atHead?: boolean;
  participate?: Participation;
  /// Each conversation's in-flight generation (live mode only): the open room renders its own at
  /// the transcript tail, so the operator watches the deliberation arrive rather than a silence.
  progress?: ReadonlyMap<string, InFlightGeneration>;
}) {
  const names = nameById(replica.memories(""));
  const convNames = conversationNameById(replica.conversations());
  // The bare handles a user can type in the "you are" field, sourced from `participant_identities`
  // so the `@platform` disambiguation suffix never surfaces as a separate entry.
  const personHandles = replica.participantIds(DIRECT_PLATFORM);
  const conversations = buildConversations(
    events.filter((event) => event.seq <= cursor),
    names,
  );
  const modelCalls = {
    ...deriveContextDebug(events, cursor),
    // The wasm-side digest verification: the reconstruction re-hashed with the recorder's own
    // serialization and compared against the digest stamped at send time.
    digestBySeq: new Map(replica.requestDigests().map((check) => [check.seq, check.status])),
  };
  // The open room rides in the URL (`?room`), so it deep-links, survives a view switch, and moves
  // with browser back and forward like the rest of the stream's state.
  const [searchParams, setSearchParams] = useSearchParams();
  const selectedKey = searchParams.get("room");
  const [draftRoom, setDraftRoom] = useState("");
  // A room the operator named but has not sent to yet — held as its own locator rather than packed
  // into a key, so it survives until its first message creates it on the log.
  const [pendingRoom, setPendingRoom] = useState<ConversationLocator | null>(null);

  // Every channel — participant rooms and the operator/imprint room alike — collected and grouped by
  // context. The operator/imprint room is offered live even before its first message; once the real
  // conversation exists on the log it is already in `known`, so the placeholder is only added then.
  const operator = conversations.find((conversation) => conversation.platform === "operator");
  const operatorPlaceholder: Channel | null = !operator && participate ? imprintChannel() : null;

  // The pending room shows until a real conversation of the same locator appears on the log.
  const known = conversations.map(toChannel);
  const pending =
    pendingRoom && !known.some((channel) => channel.key === localKey(pendingRoom))
      ? newRoomChannel(pendingRoom)
      : null;
  const all = [
    ...(pending ? [pending] : []),
    ...known,
    ...(operatorPlaceholder ? [operatorPlaceholder] : []),
  ];
  const groups = groupChannels(all);

  // Every folded turn by id, for the reference chips: the moment, the room that holds it, and up to
  // two neighbors either side for the hover preview. Built from the cursor-filtered conversations,
  // so an id past the timeline cursor reads as unknown — matching the deep-link semantics.
  const refTargets = new Map<string, TurnRefTarget>();
  for (const conversation of conversations) {
    const roomKey = channelKey(conversation.platform, conversation.scopePath);
    conversation.turns.forEach((turn, index) => {
      refTargets.set(turn.turnId, {
        turn,
        roomKey,
        window: conversation.turns.slice(Math.max(0, index - 2), index + 3),
        focusIndex: Math.min(index, 2),
      });
    });
  }

  // A deep link to a moment (`?turn=<id>`, minted by a turn's timestamp anchor) resolves to the
  // room that holds the turn — the console folds the whole log, so the turn's position is found
  // client-side. An explicit `?room` still wins (the linked room only stands in for a missing one),
  // and an id no folded conversation holds — unknown, or past the timeline cursor — leaves
  // `linkedChannel` null, which the transcript surfaces as a quiet notice rather than a crash.
  const linkedTurnId = searchParams.get("turn");
  const linkedChannel = linkedTurnId
    ? (known.find((channel) =>
        channel.conversation?.turns.some((turn) => turn.turnId === linkedTurnId),
      ) ?? null)
    : null;
  // Without an explicit `?room`, the default room is *pinned to the first resolution* rather than
  // recomputed per render: the channel list is sorted by activity, so a default that followed it
  // would yank the reader to whichever room last received a message. Landing on the busiest room is
  // right once, at open; after that the reader moves rooms only by choosing (the pulse in the list
  // shows where the action is). Set during render (React's reset-on-prop-change pattern), so the
  // pin lands in the same pass the first channel appears.
  const [defaultKey, setDefaultKey] = useState<string | null>(null);
  const first = groups[0]?.channels[0] ?? null;
  if (defaultKey === null && first) setDefaultKey(first.key);
  const selected =
    all.find((channel) => channel.key === selectedKey) ??
    linkedChannel ??
    pending ??
    all.find((channel) => channel.key === defaultKey) ??
    first ??
    null;

  // The rooms with a generation in flight, marked in the channel list (the pulse in the sidebar,
  // text in the mobile dropdown) so a stable selection still shows where the agent is working.
  const workingKeys = new Set(
    progress
      ? all
          .filter((channel) => channel.conversation && progress.has(channel.conversation.id))
          .map((channel) => channel.key)
      : [],
  );

  function selectRoom(key: string) {
    setSearchParams(
      (prev) => {
        const updated = new URLSearchParams(prev);
        updated.set("room", key);
        // Choosing a room is a navigation act of its own — the turn link has done its job, so it
        // does not follow along to highlight a moment in a room it never pointed at.
        updated.delete("turn");
        return updated;
      },
      { replace: true },
    );
  }

  function startRoom() {
    const name = draftRoom.trim();
    if (!name || hasScopeChar(name)) return;
    const locator = { platform: DIRECT_PLATFORM, scope_path: name };
    setPendingRoom(locator);
    selectRoom(localKey(locator));
    setDraftRoom("");
  }

  return (
    <ModelCalls.Provider value={modelCalls}>
      <Names.Provider value={names}>
        <ConversationNames.Provider value={convNames}>
          <TurnRefs.Provider value={refTargets}>
            {/* The transcript-and-rooms grid — the room list sits to the right, so the horizontal read
            is content first, navigation second (and the eval frame's scenario rail keeps the left
            edge to itself). The docked composer below mirrors it, so keep the two template strings
            in step. On mobile the DOM order stands (dropdown above the transcript); md moves the
            list to the second column via order. */}
            <div className="grid grid-cols-1 gap-1 md:grid-cols-[1fr_12rem] md:gap-8">
              <div className="md:sticky md:top-4 md:order-last md:self-start">
                <aside className="hidden flex-col gap-4 md:flex">
                  {participate && (
                    <RoomControls
                      sender={participate.sender}
                      onSenderChange={participate.setSender}
                      draftRoom={draftRoom}
                      onDraftRoomChange={setDraftRoom}
                      onCreateRoom={startRoom}
                      personHandles={personHandles}
                    />
                  )}

                  {groups.length === 0 ? (
                    <p className="font-mono text-2xs text-ink-faint">no conversations yet</p>
                  ) : (
                    <div className="flex flex-col gap-4">
                      {groups.map((group) => (
                        <div key={group.key} className="flex flex-col gap-1.5">
                          <Eyebrow>{group.key}</Eyebrow>
                          <nav className="flex flex-col gap-0.5">
                            {group.channels.map((channel) => (
                              <ChannelLink
                                key={channel.key}
                                channel={channel}
                                active={channel.key === selected?.key}
                                working={workingKeys.has(channel.key)}
                                onSelect={() => selectRoom(channel.key)}
                              />
                            ))}
                          </nav>
                        </div>
                      ))}
                    </div>
                  )}
                </aside>

                {/* On mobile the list collapses to a dropdown and the identity-and-new-room controls
                fold behind a disclosure, so the transcript owns the screen. */}
                <div className="flex flex-col gap-3 md:hidden">
                  <ChannelSelect
                    groups={groups}
                    selectedKey={selected?.key ?? null}
                    working={workingKeys}
                    onSelect={selectRoom}
                  />
                  {participate && (
                    <MobileRoomControls
                      sender={participate.sender}
                      onSenderChange={participate.setSender}
                      draftRoom={draftRoom}
                      onDraftRoomChange={setDraftRoom}
                      onCreateRoom={startRoom}
                      personHandles={personHandles}
                    />
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
                  atHead={atHead}
                  channel={selected}
                  inflight={
                    (selected.conversation && progress?.get(selected.conversation.id)) || null
                  }
                  participate={participate}
                  unknownTurn={
                    linkedTurnId !== null && linkedChannel === null ? linkedTurnId : null
                  }
                />
              ) : (
                <div className="py-16 text-center text-sm text-ink-faint">
                  {participate
                    ? "Name a conversation to start one."
                    : "No conversations in this run."}
                </div>
              )}
            </div>
          </TurnRefs.Provider>
        </ConversationNames.Provider>
      </Names.Provider>
    </ModelCalls.Provider>
  );
}
