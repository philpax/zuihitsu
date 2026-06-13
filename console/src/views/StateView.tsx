import { useState } from "react";

import type { Replica } from "../lib/replica.ts";
import { MemoryBrowser } from "../components/MemoryBrowser.tsx";

/// The State view: the materialized graph as it stands at the run's head, browsed memory by memory.
/// The fold never moves here — that is the Time-travel view's job — so the shared browser is used
/// directly, with selection held locally.
export function StateView({ replica }: { replica: Replica }) {
  const [selected, setSelected] = useState<string | null>(null);
  return <MemoryBrowser replica={replica} selected={selected} onSelect={setSelected} />;
}
