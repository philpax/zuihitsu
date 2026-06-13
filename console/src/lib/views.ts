/// The console's views. `scope: "package"` views read the whole eval package; `scope: "run"` views
/// operate on a single selected run's materialized graph. `ready` marks those that are wired —
/// the rest stand in the nav as the shape of what is coming, in the plan's build order.
export const VIEWS = [
  { id: "scenarios", label: "Scenarios", scope: "package", ready: true },
  { id: "state", label: "State", scope: "run", ready: true },
  { id: "conversation", label: "Conversation", scope: "run", ready: true },
  { id: "events", label: "Events", scope: "run", ready: true },
] as const;

export type ViewId = (typeof VIEWS)[number]["id"];
