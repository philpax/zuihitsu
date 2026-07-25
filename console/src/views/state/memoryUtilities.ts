import type { MemoryId } from "@zuihitsu/wire/types/MemoryId.ts";
import type { MemoryView } from "@zuihitsu/wire/types/MemoryView.ts";
import { groupBy } from "../../lib/format/collections.ts";

/// A `same_as` identity class as the sidebar renders it: the canonical primary at the head, and the
/// other live members (the platform stubs and the like) nested beneath, each still selectable. A memory
/// in no class forms a cluster of one, with no members — a plain leaf, as before.
export type MemoryCluster = {
  primary: MemoryView;
  members: MemoryView[];
};

/// Cluster memories by their `same_as` class, keyed by each memory's canonical primary id (sourced from
/// `replica.memoryClasses()`). The head of every cluster is the class primary the backend resolved — the
/// operator's designation, else the earliest member by ULID — so the console never clusters under a
/// different canonical member than the agent's own reads collapse to. When the true primary is filtered
/// out of `memories` (a search narrowed the list to a stub), the first surviving member stands in as the
/// head so the class still shows. Clusters are ordered by their head's name; members follow by name.
export function clusterByClass(
  memories: MemoryView[],
  primaryOf: Map<MemoryId, MemoryId>,
): MemoryCluster[] {
  const groups = groupBy(memories, (memory) => primaryOf.get(memory.id) ?? memory.id);
  return groups
    .map(([primaryId, items]) => {
      const sorted = [...items].sort((a, b) => a.name.localeCompare(b.name));
      const headIndex = Math.max(
        0,
        sorted.findIndex((memory) => memory.id === primaryId),
      );
      return {
        primary: sorted[headIndex],
        members: sorted.filter((_, index) => index !== headIndex),
      };
    })
    .sort((a, b) => a.primary.name.localeCompare(b.primary.name));
}

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
