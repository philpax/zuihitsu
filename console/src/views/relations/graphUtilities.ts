import { type NodeObject } from "react-force-graph-2d";

import type { RelationView } from "../../lib/model/graph.ts";
import type { MemoryGraph, MemoryGraphNode } from "../../lib/model/memoryGraph.ts";

/// World-space sizes for the graph canvas, gathered so the whole drawing scales from one place.
/// These are world units (not screen pixels), so they scale with the camera zoom.
export const SIZES = {
  node: {
    fontSize: 2.75,
    padX: 1.25,
    padY: 0.5,
    relSize: 1.25,
    strokeWidth: 0.25,
  },
  badge: {
    radius: 1.5,
    fontSize: 2,
  },
  link: {
    width: 1,
    arrowLength: 1.5,
    dash: [1.5, 1.5] as const,
    labelFontSize: 2,
    labelPadX: 0.5,
    labelPadY: 0.5,
  },
};

/// Splice expanded virtual nodes' members into the graph as satellite nodes. The class node stays
/// (so inter-class edges keep their anchor); its members appear as nodes linked to it by `same`
/// edges drawn as undirected dashed lines. No positions are seeded — the force layout's link force
/// pulls the members toward their class node, settling them as a cluster.
export function expandVirtualNodes(graph: MemoryGraph, expanded: Set<string>): MemoryGraph {
  if (expanded.size === 0) return graph;
  const nodes: MemoryGraphNode[] = [...graph.nodes];
  const links = [...graph.links];
  for (const node of graph.nodes) {
    if (!node.members || !expanded.has(node.id)) continue;
    for (const member of node.members) {
      nodes.push({ id: member, namespace: namespaceOf(member) });
      links.push({ source: node.id, target: member, relation: "same as", same: true });
    }
  }
  return { nodes, links };
}

export function isVirtual(node: NodeObject): node is NodeObject & { members: string[] } {
  return (
    Array.isArray((node as MemoryGraphNode).members) &&
    (node as MemoryGraphNode).members!.length > 0
  );
}

export function nodeLabel(node: NodeObject): string {
  if (isVirtual(node)) {
    return `${node.id} — ${node.members.join(", ")}`;
  }
  return String(node.id);
}

/// The bounding box for a pill-shaped node, in world-space units so it scales with the camera.
export function nodeShape(
  node: NodeObject,
  ctx: CanvasRenderingContext2D,
): { x: number; y: number; w: number; h: number; r: number } {
  const fontSize = SIZES.node.fontSize;
  ctx.font = `${fontSize}px ui-monospace, monospace`;
  const textWidth = ctx.measureText(String(node.id)).width;
  const padX = SIZES.node.padX;
  const padY = SIZES.node.padY;
  const w = textWidth + padX * 2;
  const h = fontSize + padY * 2;
  return {
    x: (node.x ?? 0) - w / 2,
    y: (node.y ?? 0) - h / 2,
    w,
    h,
    r: h / 2,
  };
}

export function cardinalityLabel(relation: RelationView): string {
  return `${relation.from_card}→${relation.to_card}`;
}

export function namespaceOf(name: string): string {
  const slash = name.indexOf("/");
  return slash === -1 ? name : name.slice(0, slash);
}

/// A stable color for a relation name, derived from a hash so the legend swatch and the graph edge
/// match without a hand-maintained palette. `same` edges fall back to the sage accent, since they are
/// identity plumbing rather than a typed relation with a registry entry.
export function relationColor(name: string, fallback?: string): string {
  if (fallback !== undefined) {
    // The canvas calls pass the palette's sage as the fallback for `same` edges.
    if (name === "same as") return fallback;
  }
  let hash = 0;
  for (let i = 0; i < name.length; i++) {
    hash = (hash << 5) - hash + name.charCodeAt(i);
    hash |= 0;
  }
  const hue = Math.abs(hash) % 360;
  // A moderate saturation and lightness that sits comfortably on the warm paper ground.
  return `hsl(${hue}, 55%, 45%)`;
}

/// Read the Japandi tokens off the document once, so the canvas (which can only take concrete colors)
/// stays in step with the design tokens rather than hard-coding hexes.
let cachedPalette: {
  ink: string;
  inkSoft: string;
  clay: string;
  sage: string;
  paper: string;
} | null = null;
export function readPalette() {
  if (!cachedPalette) {
    const root = getComputedStyle(document.documentElement);
    const value = (name: string) => root.getPropertyValue(name).trim();
    cachedPalette = {
      ink: value("--color-ink"),
      inkSoft: value("--color-ink-soft"),
      clay: value("--color-clay"),
      sage: value("--color-sage"),
      paper: value("--color-paper"),
    };
  }
  return cachedPalette;
}
