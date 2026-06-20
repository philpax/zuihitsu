/// The timeline-scoped views over an event stream, shared by the eval and agent frames — the nav
/// tabs, and the set the eval frame validates a `:view` URL segment against. Kept apart from the
/// workspace component so both can import it without dragging a component across a module boundary.
export const STREAM_VIEWS = [
  { id: "conversation", label: "Conversation" },
  { id: "state", label: "State" },
  { id: "graph", label: "Graph" },
  { id: "agenda", label: "Agenda" },
  { id: "events", label: "Events" },
  { id: "compare", label: "Time-travel" },
] as const;

export type StreamViewId = (typeof STREAM_VIEWS)[number]["id"];
