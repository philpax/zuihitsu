import type { MemoryView } from "@zuihitsu/wire/types/MemoryView.ts";
import { groupBy } from "../../lib/format/collections.ts";

/// Group memories by their namespace prefix (`person/dave` → `person`), `self` standing alone, with
/// `self` first and the rest alphabetical — a stable, scannable order.
export function groupByNamespace(memories: MemoryView[]): Array<[string, MemoryView[]]> {
  const namespaceOf = (name: string) => {
    const slash = name.indexOf("/");
    return slash === -1 ? name : name.slice(0, slash);
  };
  return groupBy(memories, (memory) => namespaceOf(memory.name)).sort(([a], [b]) => {
    if (a === "self") return -1;
    if (b === "self") return 1;
    return a.localeCompare(b);
  });
}

export function leafName(name: string, namespace: string): string {
  return name === namespace ? name : name.slice(namespace.length + 1);
}
