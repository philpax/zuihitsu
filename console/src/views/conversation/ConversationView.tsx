import { useState } from "react";

import type { Event } from "@zuihitsu/wire/types/Event.ts";
import { type Replica } from "../../lib/replica/replica.ts";
import { useNavigate } from "../../lib/nav/historyContext.ts";
import { Redirect } from "../../lib/nav/history.tsx";
import { useStream } from "../../lib/nav/useStreamLocation.ts";
import { nameById } from "../../lib/model/labels.ts";
import type { InFlightGeneration } from "../../lib/model/inflight.ts";
import type { ConversationLocator } from "@zuihitsu/wire/types/ConversationLocator.ts";
import { buildConversations } from "../../lib/model/conversation.ts";
import { conversationNameById } from "../../lib/model/conversationNameById.ts";
import { deriveContextDebug } from "../../lib/model/contextDebug.ts";
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
  mostRecentlyUpdated,
  newRoomChannel,
  toChannel,
} from "./channelUtilities.ts";
import { ChannelLink, ChannelSelect, MobileRoomControls, RoomControls } from "./Channels.tsx";
import { Room } from "./Room.tsx";

export {
  ConversationNames,
  ModelCalls,
  Names,
  type Participation,
} from "./conversationContexts.ts";
import {
  CanonicalNames,
  EventsBySeq,
  ModelCalls,
  Names,
  ConversationNames,
  type Participation,
} from "./conversationContexts.ts";

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
  const memories = replica.memories("");
  const names = nameById(memories);
  // Each memory's canonical display identity: its `same_as` class primary's handle, resolved through
  // the replica so a snowflake stub renders as the person it is (the recorded handle dims beside it).
  const canonicalNames = new Map(
    memories.map((memory) => [memory.id, replica.resolveMemRef(memory.id)?.handle ?? memory.name]),
  );
  const conversationList = replica.conversations();
  const convNames = conversationNameById(conversationList);
  // The graph's live conversations: a room deleted via `delete-memory` is already dropped from the
  // projection, so the transcript below shows only conversations the graph still holds.
  const liveConversationIds = new Set(conversationList.map((conv) => conv.id));
  // The bare handles a user can type in the "you are" field, sourced from `participant_identities`
  // so the `@platform` disambiguation suffix never surfaces as a separate entry.
  const personHandles = replica.participantIds(DIRECT_PLATFORM);
  const conversations = buildConversations(
    events.filter((event) => event.seq <= cursor),
    names,
    liveConversationIds,
  );
  const modelCalls = {
    ...deriveContextDebug(events, cursor),
    // The wasm-side digest verification: the reconstruction re-hashed with the recorder's own
    // serialization and compared against the digest stamped at send time.
    digestBySeq: new Map(replica.requestDigests().map((check) => [check.seq, check.status])),
  };
  // Every event by its seq, so a turn can surface (and expand) the `ConversationTurn` record behind it.
  const eventsBySeq = new Map(events.map((event) => [event.seq, event]));
  // The open room rides in the URL as the location's selection segment, so it deep-links, survives a view
  // switch, and moves with browser back and forward like the rest of the stream's state (each room
  // switch is a `push`). `?turn` stays a query — it is a highlight, not a room selection.
  const navigate = useNavigate();
  const { selection: selectedKey, search, seq, link } = useStream();
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
  const linkedTurnId = search.turn ?? null;
  const linkedChannel = linkedTurnId
    ? (known.find((channel) =>
        channel.conversation?.turns.some((turn) => turn.turnId === linkedTurnId),
      ) ?? null)
    : null;
  // Without an explicit `?room`, the default is the most-recently-updated conversation — the one whose
  // latest turn has the highest seq — reflected into the address bar just below so the open room deep-
  // links and survives a view switch. A `?turn` deep link resolves its own room (`linkedChannel`), so
  // it is not overridden here.
  const first = groups[0]?.channels[0] ?? null;
  const mostRecent = mostRecentlyUpdated(all);
  const selected =
    all.find((channel) => channel.key === selectedKey) ??
    linkedChannel ??
    pending ??
    mostRecent ??
    first ??
    null;
  // The URL sync: when no room is named and none is deep-linked, replace the location with the most-
  // recently-updated room so the address bar carries the open room. The room already renders from the
  // fallback above, so this only rewrites the URL — no blank redirect frame — and once it lands the
  // selection drives the view (so a later message to another room no longer moves the reader). No
  // conversations yet ⇒ no target ⇒ the empty state stands.
  const defaultRoom =
    selectedKey === null && linkedTurnId === null && mostRecent ? mostRecent : null;

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
    // Choosing a room is a navigation act of its own, so it pushes a history entry (back returns to
    // the prior room). The `seq` cursor rides along when pinned; the `turn` highlight does not — the
    // turn link has done its job, so it does not follow to highlight a moment in a room it never
    // pointed at.
    navigate(link.conversation({ room: key, seq }));
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
        <CanonicalNames.Provider value={canonicalNames}>
          <ConversationNames.Provider value={convNames}>
            <EventsBySeq.Provider value={eventsBySeq}>
              <TurnRefs.Provider value={refTargets}>
                {defaultRoom && <Redirect to={link.conversation({ room: defaultRoom.key, seq })} />}
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
            </EventsBySeq.Provider>
          </ConversationNames.Provider>
        </CanonicalNames.Provider>
      </Names.Provider>
    </ModelCalls.Provider>
  );
}
