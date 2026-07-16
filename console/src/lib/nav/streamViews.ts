/// The view vocabulary — every value the `:view` segment may take, typed so the tabs, the route
/// builders, and the frames validating a URL share one set rather than bare strings. [`STREAM_VIEWS`]
/// are timeline-scoped and shared by both frames; [`AGENT_VIEW_IDS`] are the agent-only tabs (Lua
/// console, prompts, settings) added in the live frame. Together they are [`ViewId`].
export const STREAM_VIEWS = [
  { id: "conversation", label: "Conversation" },
  { id: "background", label: "Background" },
  { id: "state", label: "State" },
  { id: "relations", label: "Relations" },
  { id: "agenda", label: "Agenda" },
  { id: "events", label: "Events" },
  { id: "compare", label: "Time-travel" },
] as const;

export type StreamViewId = (typeof STREAM_VIEWS)[number]["id"];

/// The agent-only views, valid `:view` segments in the live frame but absent from the eval frame.
/// Only their ids live here — the tab labels and the live nodes they render are assembled with the
/// connection in `LiveShell`.
export const AGENT_VIEW_IDS = ["console", "prompts", "settings"] as const;

export type AgentViewId = (typeof AGENT_VIEW_IDS)[number];

/// Every `:view` value across both frames.
export type ViewId = StreamViewId | AgentViewId;

/// Whether a raw `:view` segment names a timeline-scoped stream view — the guard the eval frame uses
/// to reject a `:view` its (extra-view-less) nav does not carry.
export function isStreamViewId(value: string | undefined): value is StreamViewId {
  return STREAM_VIEWS.some((entry) => entry.id === value);
}

/// A raw `:view` segment narrowed to a [`ViewId`], or `undefined` if it names no known view (a bare
/// or malformed segment), so a reader gets the typed union rather than an unchecked string.
export function asViewId(value: string | undefined): ViewId | undefined {
  if (isStreamViewId(value)) return value;
  return (AGENT_VIEW_IDS as readonly string[]).includes(value ?? "")
    ? (value as AgentViewId)
    : undefined;
}
