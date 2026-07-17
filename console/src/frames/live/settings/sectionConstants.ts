/// The view's three concerns, each its own section: the agent's behavioral settings (editable, live),
/// the environmental TOML config it booted from (read-only), and maintenance actions. The open
/// section rides in the URL as the location's selection segment, so it deep-links, survives a view switch, and
/// moves with browser back and forward.
export const SECTIONS = [
  { id: "settings", label: "Settings" },
  { id: "environment", label: "Environment" },
  { id: "maintenance", label: "Maintenance" },
] as const;
export type SectionId = (typeof SECTIONS)[number]["id"];
