import { createContext } from "react";

/// The workspace's bottom dock: a slot in the sticky footer, above the timeline, where a view can
/// float its own controls — the conversation composer — so they stay reachable wherever the page is
/// scrolled. The workspace provides the dock element; `Docked` (views/conversation/Docked.tsx) teleports its
/// children there.
export const DockContext = createContext<HTMLElement | null>(null);
