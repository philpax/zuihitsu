import { useContext, type ReactNode } from "react";
import { createPortal } from "react-dom";

import { DockContext } from "../lib/dock.ts";

/// Teleport children into the workspace's bottom dock (see `lib/dock.ts`). Nothing renders until
/// the dock ref lands (one frame after mount), which also covers hosts with no dock.
export function Docked({ children }: { children: ReactNode }) {
  const dock = useContext(DockContext);
  if (!dock) return null;
  return createPortal(children, dock);
}
