import type { ConversationLocator } from "../../types/ConversationLocator.ts";
import { type ConversationModel, type SessionModel } from "../../lib/model/conversation.ts";
import { formatDateTime } from "../../lib/format/format.ts";
import { LabeledDivider } from "../../components/primitives.tsx";

/// A conversation as the sidebar and composer see it: a stable key (its locator, so a pending room
/// and the real one it becomes share an entry), a label, the locator to address it, the authority
/// composing into it carries, and the folded model (`null` until it exists on the log).
export interface Channel {
  key: string;
  label: string;
  locator: ConversationLocator;
  authority: "operator" | "participant";
  conversation: ConversationModel | null;
}

/// The context a room belongs to — the platform segment of its `context/*` memory name
/// (`context/operator:imprint` → `operator`), falling back to the locator's platform when no context
/// memory exists yet (a pending room). This is the grouping key for the sidebar: rooms under the same
/// context — `operator`, `console`, `discord` — cluster together rather than the prior split that
/// singled out `operator` and left the rest in one flat list.
export function contextGroup(channel: Channel): string {
  const name = channel.conversation?.contextName;
  if (name) {
    const stripped = name.startsWith("context/") ? name.slice("context/".length) : name;
    const colon = stripped.indexOf(":");
    return colon === -1 ? stripped : stripped.slice(0, colon);
  }
  return channel.locator.platform;
}

/// Channels grouped by context, sorted most-recently-used first within each group, and the groups
/// themselves ordered by their most active channel — so the room with the freshest activity and its
/// siblings surface to the top.
export interface ChannelGroup {
  key: string;
  channels: Channel[];
}

export function groupChannels(channels: Channel[]): ChannelGroup[] {
  const byGroup = new Map<string, Channel[]>();
  for (const channel of channels) {
    const key = contextGroup(channel);
    const list = byGroup.get(key);
    if (list) list.push(channel);
    else byGroup.set(key, [channel]);
  }
  const groups = [...byGroup.entries()].map(([key, list]) => ({
    key,
    channels: list.sort((a, b) => lastActivity(b.conversation) - lastActivity(a.conversation)),
  }));
  // Groups ordered by their most active channel, so the freshest context sits at the top.
  return groups.sort(
    (a, b) =>
      Math.max(...b.channels.map((c) => lastActivity(c.conversation))) -
      Math.max(...a.channels.map((c) => lastActivity(c.conversation))),
  );
}

export function toChannel(conversation: ConversationModel): Channel {
  return {
    key: channelKey(conversation.platform, conversation.scopePath),
    // The bare scope path, not the context memory name — the group header already names the context,
    // so the label should stay as what the user typed (e.g. "blah"), not jump to "context/direct:blah"
    // once the context memory is minted.
    label: conversation.scopePath,
    locator: { platform: conversation.platform, scope_path: conversation.scopePath },
    authority: conversation.platform === "operator" ? "operator" : "participant",
    conversation,
  };
}

export function imprintChannel(): Channel {
  return {
    key: channelKey("operator", "imprint"),
    label: "imprint",
    locator: { platform: "operator", scope_path: "imprint" },
    authority: "operator",
    conversation: null,
  };
}

export function newRoomChannel(locator: ConversationLocator): Channel {
  return {
    key: localKey(locator),
    label: locator.scope_path,
    locator,
    authority: "participant",
    conversation: null,
  };
}

export function channelKey(platform: string, scopePath: string): string {
  return localKey({ platform, scope_path: scopePath });
}

/// A stable selection key for a room — its locator, never parsed back, so the separator is only a
/// display concern and a room name may contain anything.
export function localKey(locator: ConversationLocator): string {
  return `${locator.platform} · ${locator.scope_path}`;
}

/// Whether a value carries a scope character (`/` for `person/*`, `:` for `context/*`) — the sign
/// the user typed a full path when a bare name is expected. Used to warn inline and gate submission.
export function hasScopeChar(value: string): boolean {
  return value.includes("/") || value.includes(":");
}

export function lastActivity(conversation: ConversationModel | null): number {
  return conversation ? conversation.turns.reduce((max, turn) => Math.max(max, turn.seq), 0) : 0;
}

/// The seam between conversations in one context. A context can hold several conversations over time
/// — each opens its own session with a freshly composed brief — and previously the only sign of a new
/// one was the brief reappearing. This draws the boundary plainly: a labelled rule with the date the
/// conversation opened, reading "conversation" at the context's first, "new conversation" at each one
/// after, and "re-briefed · compaction" when a session reopened by re-segmenting the last rather than
/// starting fresh.
export function SessionDivider({ session, first }: { session: SessionModel; first: boolean }) {
  const label = session.compaction
    ? "re-briefed · compaction"
    : first
      ? "conversation"
      : "new conversation";
  return (
    <LabeledDivider
      className={(first ? "mb-4 " : "my-4 ") + (session.compaction ? "text-clay" : "text-ink-soft")}
    >
      <span className="uppercase tracking-widest">{label}</span>
      <span className="text-ink-faint">{formatDateTime(session.startedAt)}</span>
    </LabeledDivider>
  );
}
