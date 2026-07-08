import type { MemoryId } from "../../types/MemoryId.ts";
import type { Teller } from "../../types/Teller.ts";
import type { Visibility } from "../../types/Visibility.ts";
import type { Replica } from "../replica/replica.ts";

/// The memory graph at the current fold, shaped for a force-directed layout: a node per memory and an
/// edge per relation, plus the undirected `same_as` edges that bind a memory's identity class. Names
/// are the node ids (the State view addresses memories by name), so a click can open one directly.
export interface MemoryGraphNode {
  id: string;
  namespace: string;
  /// The members of a collapsed `same_as` identity class. Present only on virtual nodes produced by
  /// `collapseSameAs` (where `length > 1`); the representative name is `id` sans the count suffix.
  members?: string[];
}

export interface MemoryGraphLink {
  source: string;
  target: string;
  relation: string;
  /// A `same_as` identity edge rather than a typed relation — drawn undirected and distinct.
  same: boolean;
  /// The link's audience posture, surfaced so the relations view can mark non-public links.
  visibility: Visibility;
  /// Who asserted the relationship, if known — `null` for structural links with no teller.
  told_by: Teller | null;
  /// The context memory (room) the link was asserted in, if any.
  told_in: MemoryId | null;
}

export interface MemoryGraph {
  nodes: MemoryGraphNode[];
  links: MemoryGraphLink[];
}

export function buildMemoryGraph(replica: Replica): MemoryGraph {
  const memories = replica.memories("");
  const idToName = new Map(memories.map((memory) => [memory.id, memory.name]));
  const nodes = memories.map((memory) => ({
    id: memory.name,
    namespace: namespaceOf(memory.name),
  }));

  const links: MemoryGraphLink[] = [];
  const seen = new Set<string>();
  for (const memory of memories) {
    const detail = replica.memory(memory.name);
    if (!detail) continue;

    for (const link of detail.links) {
      const source = idToName.get(link.from);
      const target = idToName.get(link.to);
      if (!source || !target) continue;
      const key = `rel-${source}-${link.relation}-${target}`;
      if (seen.has(key)) continue;
      seen.add(key);
      links.push({
        source,
        target,
        relation: link.relation,
        same: false,
        visibility: link.visibility,
        told_by: link.told_by,
        told_in: link.told_in,
      });
    }

    for (const peer of detail.class) {
      if (peer.id === memory.id) continue;
      const key = `same-${[memory.name, peer.name].sort().join("-")}`;
      if (seen.has(key)) continue;
      seen.add(key);
      links.push({
        source: memory.name,
        target: peer.name,
        relation: "same as",
        same: true,
        visibility: "Public" as Visibility,
        told_by: null,
        told_in: null,
      });
    }
  }

  return { nodes, links };
}

/// Collapse each `same_as` identity class into a single virtual node. Union-find over the `same` edges
/// computes the classes; each becomes a node whose `id` is its lexicographically smallest member
/// suffixed with the member count (e.g. `"person/dave (3)"`), carrying `members` as the single source
/// of truth for class membership. Typed edges between members of the same class are dropped
/// (intra-class), and those between classes route through their class nodes. `same` edges vanish —
/// they are subsumed by the merge. A class of one is left as its original node (no `members`).
export function collapseSameAs(graph: MemoryGraph): MemoryGraph {
  const parent = new Map<string, string>();
  function find(x: string): string {
    let root = x;
    for (let p = parent.get(root); p !== undefined && p !== root; p = parent.get(root)) {
      root = p;
    }
    // Path compression.
    let cur = x;
    for (let p = parent.get(cur); p !== undefined && p !== cur; p = parent.get(cur)) {
      parent.set(cur, root);
      cur = p;
    }
    return root;
  }
  function union(a: string, b: string) {
    const ra = find(a);
    const rb = find(b);
    if (ra === rb) return;
    const rep = ra < rb ? ra : rb;
    const other = ra < rb ? rb : ra;
    parent.set(other, rep);
  }

  for (const node of graph.nodes) parent.set(node.id, node.id);
  for (const link of graph.links) {
    if (link.same) union(link.source, link.target);
  }

  // Group node ids by their root, then order each class lexicographically so the representative is
  // stable regardless of insertion order.
  const classes = new Map<string, string[]>();
  for (const node of graph.nodes) {
    const root = find(node.id);
    const group = classes.get(root);
    if (group) group.push(node.id);
    else classes.set(root, [node.id]);
  }

  const nodeById = new Map(graph.nodes.map((node) => [node.id, node]));
  const classOf = new Map<string, string[]>(); // member name → its class members
  const collapsedNodes: MemoryGraphNode[] = [];
  for (const members of classes.values()) {
    members.sort();
    const rep = members[0];
    if (members.length === 1) {
      // Singletons stay as-is; no virtual node, no `members` field.
      const original = nodeById.get(rep);
      if (original) collapsedNodes.push(original);
      classOf.set(rep, members);
      continue;
    }
    const id = `${rep} (${members.length})`;
    const namespace = namespaceOf(rep);
    collapsedNodes.push({ id, namespace, members: members.slice() });
    for (const member of members) classOf.set(member, members);
  }

  // Route typed edges through their class nodes, dropping intra-class edges and deduping.
  const collapsedLinks: MemoryGraphLink[] = [];
  const seen = new Set<string>();
  for (const link of graph.links) {
    if (link.same) continue; // subsumed by the merge.
    const sourceMembers = classOf.get(link.source);
    const targetMembers = classOf.get(link.target);
    const sourceRep = sourceMembers ? sourceMembers[0] : link.source;
    const targetRep = targetMembers ? targetMembers[0] : link.target;
    if (sourceRep === targetRep) continue; // intra-class.
    const sourceId =
      sourceMembers && sourceMembers.length > 1
        ? `${sourceRep} (${sourceMembers.length})`
        : sourceRep;
    const targetId =
      targetMembers && targetMembers.length > 1
        ? `${targetRep} (${targetMembers.length})`
        : targetRep;
    const key = `${sourceId}-${link.relation}-${targetId}`;
    if (seen.has(key)) continue;
    seen.add(key);
    collapsedLinks.push({
      source: sourceId,
      target: targetId,
      relation: link.relation,
      same: false,
      visibility: link.visibility,
      told_by: link.told_by,
      told_in: link.told_in,
    });
  }

  return { nodes: collapsedNodes, links: collapsedLinks };
}

/// Filter the graph to the selected relations' typed edges and the nodes they touch. An empty set
/// leaves the graph unchanged (all relations). `same` edges are never kept by the relation filter —
/// they belong to the collapse step, which runs before this in the pipeline.
export function filterByRelations(graph: MemoryGraph, relations: Set<string>): MemoryGraph {
  if (relations.size === 0) return graph;
  const links = graph.links.filter((link) => !link.same && relations.has(link.relation));
  const touched = new Set<string>();
  for (const link of links) {
    touched.add(link.source);
    touched.add(link.target);
  }
  const nodes = graph.nodes.filter((node) => touched.has(node.id));
  return { nodes, links };
}

function namespaceOf(name: string): string {
  const slash = name.indexOf("/");
  return slash === -1 ? name : name.slice(0, slash);
}
