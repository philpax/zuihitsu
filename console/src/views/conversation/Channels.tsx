import { useState } from "react";

import {
  Disclosure,
  Eyebrow,
  Hint,
  Select,
  TextInput,
  WorkingPulse,
} from "../../components/primitives.tsx";
import { type Channel, type ChannelGroup, hasScopeChar } from "./channelUtilities.tsx";

/// The mobile face of the conversation list: a native dropdown grouped by context (one optgroup per
/// context, its channels most-recently-used first) so the transcript owns the screen. Hidden once the
/// sidebar fits (`md`).
export function ChannelSelect({
  groups,
  selectedKey,
  working,
  onSelect,
}: {
  groups: ChannelGroup[];
  selectedKey: string | null;
  /// The keys of channels with a generation in flight; a native option cannot carry the pulse, so
  /// these are marked in text.
  working?: ReadonlySet<string>;
  onSelect: (key: string) => void;
}) {
  return (
    <Select
      value={selectedKey ?? ""}
      onChange={(event) => onSelect(event.target.value)}
      aria-label="Choose a conversation"
    >
      {groups.map((group) => (
        <optgroup key={group.key} label={group.key}>
          {group.channels.map((channel) => (
            <option key={channel.key} value={channel.key}>
              {channel.label}
              {working?.has(channel.key) ? " · deliberating" : ""}
            </option>
          ))}
        </optgroup>
      ))}
    </Select>
  );
}

/// The sidebar's live controls — the handle you speak under and the field to open a new conversation —
/// shared by the desktop sidebar and the mobile stack so they stay in sync. Enter on the room name
/// opens the conversation. The handle input offers a native autocomplete from the agent's known
/// `person/*` memories, so a returning participant can pick an existing handle rather than typing it
/// from scratch (and risking a new stub).
export function RoomControls({
  sender,
  onSenderChange,
  draftRoom,
  onDraftRoomChange,
  onCreateRoom,
  personHandles,
}: {
  sender: string;
  onSenderChange: (value: string) => void;
  draftRoom: string;
  onDraftRoomChange: (value: string) => void;
  onCreateRoom: () => void;
  personHandles: string[];
}) {
  const senderScoped = hasScopeChar(sender);
  const roomScoped = hasScopeChar(draftRoom.trim());
  return (
    <div className="flex flex-col gap-3">
      <label className="flex flex-col gap-1.5">
        <Eyebrow>you are</Eyebrow>
        <TextInput
          value={sender}
          onChange={(event) => onSenderChange(event.target.value)}
          list="person-handles"
          placeholder="a handle"
        />
        <datalist id="person-handles">
          {personHandles.map((handle) => (
            <option key={handle} value={handle} />
          ))}
        </datalist>
        {senderScoped ? (
          <Hint tone="error">
            a bare name, not a memory path — drop the “{sender.slice(0, sender.indexOf("/"))}/”
          </Hint>
        ) : (
          <Hint>person · a name, not a memory id</Hint>
        )}
      </label>

      <label className="flex flex-col gap-1.5">
        <Eyebrow>new conversation</Eyebrow>
        <TextInput
          value={draftRoom}
          onChange={(event) => onDraftRoomChange(event.target.value)}
          onKeyDown={(event) => event.key === "Enter" && !roomScoped && onCreateRoom()}
          placeholder="a room name"
        />
        {roomScoped ? (
          <Hint tone="error">
            a bare name, not a context path — drop the “
            {draftRoom.trim().slice(0, draftRoom.trim().indexOf(":"))}:”
          </Hint>
        ) : (
          <Hint>direct · a name, not a context path</Hint>
        )}
      </label>
    </div>
  );
}

/// The mobile face of the live controls: folded behind a disclosure whose summary names who you are
/// speaking as (or that no one is set yet), so a phone's first screen belongs to the transcript. The
/// composer's disabled placeholder points here when a handle is still needed.
export function MobileRoomControls(props: {
  sender: string;
  onSenderChange: (value: string) => void;
  draftRoom: string;
  onDraftRoomChange: (value: string) => void;
  onCreateRoom: () => void;
  personHandles: string[];
}) {
  const [open, setOpen] = useState(false);
  return (
    <div>
      <Disclosure
        open={open}
        onToggle={() => setOpen(!open)}
        label="you"
        summary={props.sender.trim() ? `speaking as ${props.sender.trim()}` : "set who you are"}
      />
      {open && (
        <div className="mt-3">
          <RoomControls {...props} />
        </div>
      )}
    </div>
  );
}

export function ChannelLink({
  channel,
  active,
  working,
  onSelect,
}: {
  channel: Channel;
  active: boolean;
  /// True while a generation is in flight in this room — the sage pulse marks where the agent is
  /// working, so the reader can find the action without the view following it uninvited.
  working?: boolean;
  onSelect: () => void;
}) {
  return (
    <button
      onClick={onSelect}
      title={channel.label}
      className={
        "flex w-full min-w-0 items-baseline gap-2 border-l-2 py-1.5 pl-2.5 text-left text-sm transition-colors " +
        (active
          ? "border-clay font-medium text-ink"
          : "border-transparent text-ink-soft hover:text-ink") +
        (channel.conversation ? "" : " italic text-ink-faint")
      }
    >
      <span className="truncate">{channel.label}</span>
      {working && <WorkingPulse className="shrink-0 self-center" />}
    </button>
  );
}
