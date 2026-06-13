import { useState } from "react";

import type { Replica } from "../lib/replica.ts";
import { MemoryBrowser } from "../components/MemoryBrowser.tsx";

/// The State view: the materialized graph as it stands at the timeline cursor, browsed memory by
/// memory. The Shell folds the replica to `cursor`; keying the browser by it re-queries at that
/// fold, while selection is held here so it survives the remount.
export function StateView({ replica, cursor }: { replica: Replica; cursor: number }) {
  const [selected, setSelected] = useState<string | null>(null);
  return (
    <MemoryBrowser key={cursor} replica={replica} selected={selected} onSelect={setSelected} />
  );
}
