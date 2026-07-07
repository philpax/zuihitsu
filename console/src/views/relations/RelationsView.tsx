import { useEffect, useRef, useState } from "react";
import { useNavigate, useSearchParams } from "react-router-dom";
import ForceGraph2D, {
  type ForceGraphMethods,
  type LinkObject,
  type NodeObject,
} from "react-force-graph-2d";

import type { MemoryId } from "../../types/MemoryId.ts";
import type { Replica } from "../../lib/replica/replica.ts";
import type { RelationView } from "../../lib/model/graph.ts";
import {
  buildMemoryGraph,
  collapseSameAs,
  filterByRelations,
} from "../../lib/model/memoryGraph.ts";
import type { MemoryGraph, MemoryGraphNode } from "../../lib/model/memoryGraph.ts";
import { useStreamBase } from "../../lib/nav/useStreamLocation.ts";
import { statePath } from "../../lib/nav/routes.ts";
import { Checkbox } from "../../components/primitives.tsx";
import { MergeProposals } from "./MergeProposals.tsx";

/// The operator's merge-decision hook, supplied only by the live agent frame when the cursor is at the
/// head — resolving a proposal authors an operator event, which the read-only eval viewer cannot do.
export interface MergeControls {
  resolve: (from: MemoryId, to: MemoryId, accept: boolean) => Promise<void>;
}

/// World-space sizes for the graph canvas, gathered so the whole drawing scales from one place.
/// These are world units (not screen pixels), so they scale with the camera zoom.
const SIZES = {
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

/// The Relations view: the relation registry as a filterable table at the top, the force-directed
/// graph below it, and the linked-pairs list below that. The graph is the same folded materialization
/// the State view browses by memory; here it is read by edge rather than by node — a glance at how
/// the graph hangs together by relation, rather than by name.
///
/// Each relation has a stable color derived from a hash of its name, so the legend swatches and the
/// edge colors in the canvas match without a hand-maintained palette. Multiple relations may be
/// selected at once to filter the graph to their edges; selecting none shows all. The `same_as`
/// collapse (on by default) runs a union-find over the identity edges *before* relation filtering,
/// so merged memories render as one node carrying a member-count badge. Clicking a virtual node
/// expands it to show its members as a cluster; the expansion lives in view state, not the graph.
/// When relations are selected, a linked-pairs list below the graph spells out the
/// `source relation target` triples with clickable names. The selected relations, the collapse
/// toggle, and the expanded classes all ride in the URL so the view survives the cursor-keyed
/// remount and browser history.
export function RelationsView({
  replica,
  cursor,
  merge,
}: {
  replica: Replica;
  cursor: number;
  merge?: MergeControls;
}) {
  const navigate = useNavigate();
  const base = useStreamBase();
  const palette = readPalette();
  const [searchParams, setSearchParams] = useSearchParams();

  // URL state: the selected relations (empty = all), the `same_as` collapse toggle (default on),
  // and the comma-joined set of expanded virtual-node ids. Defaults are applied when the param is
  // absent so the first visit to the tab is the intended de-cluttered overview.
  const relationParam = searchParams.get("relations");
  const selected =
    relationParam && relationParam !== ""
      ? new Set(relationParam.split(",").filter(Boolean))
      : new Set<string>();
  const sameAs = searchParams.get("sameAs") !== "off";
  const expandParam = searchParams.get("expand");
  const expanded =
    expandParam && expandParam !== "" ? new Set(expandParam.split(",")) : new Set<string>();

  const relations = replica.relations().filter((relation) => relation.name !== "same_as");

  // Pipeline order matters: collapse runs on the full graph first (while `same` edges are present for
  // the union-find), then filtering keeps only the selected relations' typed edges between the
  // collapsed identity nodes. Reversing this drops `same` edges before collapse sees them, making the
  // toggle a no-op whenever a relation is selected.
  const raw = buildMemoryGraph(replica);
  const collapsed = sameAs ? collapseSameAs(raw) : raw;
  const filtered = filterByRelations(collapsed, selected);

  function toggleRelation(name: string) {
    setSearchParams(
      (prev) => {
        const updated = new URLSearchParams(prev);
        const current =
          updated.get("relations") && updated.get("relations") !== ""
            ? new Set(updated.get("relations")!.split(",").filter(Boolean))
            : new Set<string>();
        if (current.has(name)) current.delete(name);
        else current.add(name);
        if (current.size === 0) updated.delete("relations");
        else updated.set("relations", [...current].join(","));
        return updated;
      },
      { replace: true },
    );
  }

  function clearRelations() {
    setSearchParams(
      (prev) => {
        const updated = new URLSearchParams(prev);
        updated.delete("relations");
        return updated;
      },
      { replace: true },
    );
  }

  function toggleSameAs(on: boolean) {
    setSearchParams(
      (prev) => {
        const updated = new URLSearchParams(prev);
        if (on) updated.delete("sameAs");
        else updated.set("sameAs", "off");
        return updated;
      },
      { replace: true },
    );
  }

  function toggleExpand(id: string) {
    setSearchParams(
      (prev) => {
        const updated = new URLSearchParams(prev);
        const current =
          updated.get("expand") && updated.get("expand") !== ""
            ? new Set(updated.get("expand")!.split(","))
            : new Set<string>();
        if (current.has(id)) current.delete(id);
        else current.add(id);
        if (current.size === 0) updated.delete("expand");
        else updated.set("expand", [...current].join(","));
        return updated;
      },
      { replace: true },
    );
  }

  // The force-graph canvas needs explicit pixel dimensions, so measure the container it fills.
  const wrap = useRef<HTMLDivElement>(null);
  const graphRef = useRef<ForceGraphMethods | undefined>(undefined);
  const [size, setSize] = useState({ width: 0, height: 0 });
  useEffect(() => {
    const element = wrap.current;
    if (!element) return;
    const observer = new ResizeObserver((entries) => {
      const rect = entries[0].contentRect;
      setSize({ width: Math.floor(rect.width), height: Math.floor(rect.height) });
    });
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  const graphData = expandVirtualNodes(filtered, expanded);
  const proposals = replica.mergeProposals();

  return (
    <div className="flex flex-col gap-4">
      {/* The cross-platform merge proposals derived from the folded log — the operator's identity
          adjudication surface, above the relation graph the merges reshape. */}
      <MergeProposals
        proposals={proposals}
        base={base}
        cursor={cursor}
        navigate={navigate}
        onResolve={merge?.resolve}
      />

      {raw.nodes.length === 0 ? (
        <div className="py-16 text-center text-sm text-ink-faint">
          No memories to graph at this point in the log.
        </div>
      ) : (
        <>
          {/* Legend: the relation registry as a vertical table at the top. Each row is a toggle
              filter; clicking "all" clears it. `same_as` is excluded — it is identity plumbing handled
              by the collapse, and its edges are labeled "same as" (with a space), not the wire name. */}
          <RelationLegend
            relations={relations}
            selected={selected}
            onToggle={toggleRelation}
            onClear={clearRelations}
            sameAs={sameAs}
            onToggleSameAs={toggleSameAs}
          />

          <div ref={wrap} className="h-[40vh] w-full overflow-hidden border border-line bg-oat/20">
            {size.width > 0 && (
              <ForceGraph2D
                ref={graphRef}
                graphData={graphData}
                width={size.width}
                height={size.height}
                backgroundColor="rgba(0,0,0,0)"
                cooldownTicks={100}
                onEngineStop={() => graphRef.current?.zoomToFit(400, 80)}
                nodeRelSize={SIZES.node.relSize}
                nodeColor={(node: NodeObject) =>
                  isVirtual(node) ? palette.sage : node.id === "self" ? palette.clay : palette.ink
                }
                nodeLabel={(node: NodeObject) => nodeLabel(node)}
                linkColor={(link: LinkObject) => relationColor(link.relation, palette.sage)}
                linkLineDash={(link: LinkObject) => (link.same ? [...SIZES.link.dash] : null)}
                linkDirectionalArrowLength={(link: LinkObject) =>
                  link.same ? 0 : SIZES.link.arrowLength
                }
                linkDirectionalArrowRelPos={1}
                linkWidth={SIZES.link.width}
                linkLabel={(link: LinkObject) => String(link.relation)}
                linkCanvasObjectMode={() => "after"}
                linkCanvasObject={(link: LinkObject, ctx) => {
                  const { source, target } = link;
                  if (typeof source !== "object" || typeof target !== "object") return;
                  const x = ((source.x ?? 0) + (target.x ?? 0)) / 2;
                  const y = ((source.y ?? 0) + (target.y ?? 0)) / 2;
                  const fontSize = SIZES.link.labelFontSize;
                  ctx.font = `${fontSize}px ui-monospace, monospace`;
                  const width = ctx.measureText(link.relation).width;
                  const padX = SIZES.link.labelPadX;
                  const padY = SIZES.link.labelPadY;
                  // A paper chip behind the text keeps it legible where it crosses an edge or node. Drawn
                  // in world space alongside the nodes, so it scales with the camera too.
                  ctx.fillStyle = palette.paper;
                  ctx.fillRect(
                    x - width / 2 - padX,
                    y - fontSize / 2 - padY,
                    width + padX * 2,
                    fontSize + padY * 2,
                  );
                  ctx.fillStyle = relationColor(link.relation, palette.sage);
                  ctx.textAlign = "center";
                  ctx.textBaseline = "middle";
                  ctx.fillText(link.relation, x, y);
                }}
                nodeCanvasObjectMode={() => "replace"}
                nodeCanvasObject={(node: NodeObject, ctx) => {
                  const shape = nodeShape(node, ctx);
                  const stroke = isVirtual(node)
                    ? palette.sage
                    : node.id === "self"
                      ? palette.clay
                      : palette.ink;

                  // The pill: a paper fill with a hairline border, so the label reads against the edge
                  // crossings and the warm graph ground alike. Drawn in world space, so it scales with
                  // the camera — zooming in shrinks it relative to the viewport, keeping the graph at a
                  // consistent relative scale.
                  ctx.fillStyle = palette.paper;
                  ctx.strokeStyle = stroke;
                  ctx.lineWidth = SIZES.node.strokeWidth;
                  ctx.beginPath();
                  ctx.roundRect(shape.x, shape.y, shape.w, shape.h, shape.r);
                  ctx.fill();
                  ctx.stroke();

                  // The label, centered inside the pill.
                  ctx.fillStyle = palette.ink;
                  ctx.font = `${SIZES.node.fontSize}px ui-monospace, monospace`;
                  ctx.textAlign = "center";
                  ctx.textBaseline = "middle";
                  ctx.fillText(String(node.id), node.x ?? 0, node.y ?? 0);

                  // A member-count badge above a virtual node's pill, so the merge is visible at a glance.
                  if (isVirtual(node)) {
                    const bx = (node.x ?? 0) + shape.w / 2;
                    const by = (node.y ?? 0) - shape.h / 2;
                    ctx.fillStyle = palette.sage;
                    ctx.beginPath();
                    ctx.arc(bx, by, SIZES.badge.radius, 0, 2 * Math.PI);
                    ctx.fill();
                    ctx.fillStyle = palette.paper;
                    ctx.font = `${SIZES.badge.fontSize}px ui-monospace, monospace`;
                    ctx.textAlign = "center";
                    ctx.textBaseline = "middle";
                    ctx.fillText(String(node.members!.length), bx, by);
                  }
                }}
                nodePointerAreaPaint={(node: NodeObject, paintColor: string, ctx) => {
                  const shape = nodeShape(node, ctx);
                  ctx.fillStyle = paintColor;
                  ctx.beginPath();
                  ctx.roundRect(shape.x, shape.y, shape.w, shape.h, shape.r);
                  ctx.fill();
                }}
                onNodeClick={(node: NodeObject) => {
                  if (isVirtual(node)) {
                    toggleExpand(String(node.id));
                  } else {
                    navigate(statePath(base, cursor, String(node.id)));
                  }
                }}
              />
            )}
          </div>

          {/* Linked-pairs detail: the graph shows the shape; this spells out the
              `source relation target` triples, each name clickable into State. Shown for all
              relations when "all" is active, or just the selected ones when filtering. */}
          <LinkedPairs graph={filtered} base={base} cursor={cursor} navigate={navigate} />
        </>
      )}
    </div>
  );
}

/// The relation registry as a table. Each row is a toggle filter; clicking "all" clears the filter.
/// The swatch column matches the graph's edge color for the relation.

function RelationLegend({
  relations,
  selected,
  onToggle,
  onClear,
  sameAs,
  onToggleSameAs,
}: {
  relations: RelationView[];
  selected: Set<string>;
  onToggle: (name: string) => void;
  onClear: () => void;
  sameAs: boolean;
  onToggleSameAs: (on: boolean) => void;
}) {
  return (
    <nav className="flex flex-col gap-1">
      <div className="flex items-center justify-between">
        <button
          onClick={onClear}
          className={
            "border-b-2 pb-0.5 font-mono text-xs transition-colors " +
            (selected.size === 0
              ? "border-clay text-ink"
              : "border-transparent text-ink-soft hover:text-ink")
          }
        >
          all relations
        </button>
        <Checkbox checked={sameAs} onChange={onToggleSameAs} label="collapse same_as" />
      </div>
      {relations.length === 0 ? (
        <p className="py-2 font-mono text-2xs text-ink-faint">no registered relations</p>
      ) : (
        // Scrolls sideways on a narrow screen rather than crushing its fixed columns.
        <div className="overflow-x-auto">
          <table className="w-full min-w-[34rem] table-fixed border-collapse">
            <thead>
              <tr className="border-b border-line text-left font-mono text-2xs uppercase tracking-widest text-ink-faint">
                <th className="w-[20%] pb-1 pr-2 font-normal">name</th>
                <th className="w-[20%] pb-1 pr-2 font-normal">inverse</th>
                <th className="w-24 pb-1 pr-2 font-normal">card</th>
                <th className="pb-1 font-normal">description</th>
              </tr>
            </thead>
            <tbody>
              {relations.map((relation) => {
                const active = selected.has(relation.name);
                const color = relationColor(relation.name);
                return (
                  <tr
                    key={relation.name}
                    onClick={() => onToggle(relation.name)}
                    className={
                      "cursor-pointer border-l-2 align-baseline transition-colors " +
                      (active ? "border-clay" : "border-transparent hover:bg-oat/30")
                    }
                  >
                    <td className="py-1 pl-2.5 pr-2 font-mono text-xs" style={{ color }}>
                      {relation.name}
                    </td>
                    <td className="py-1 pr-2 font-mono text-2xs text-ink-faint">
                      {relation.inverse}
                    </td>
                    <td className="py-1 pr-2 font-mono text-2xs text-ink-faint">
                      {cardinalityLabel(relation)}
                    </td>
                    <td className="py-1 text-2xs leading-snug text-ink-faint">
                      {relation.description || "—"}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}
    </nav>
  );
}

/// The `source relation target` triples for the selected relations, each name a link into the State
/// view at the cursor. The colored relation name is the verb — no arrows. Virtual nodes (collapsed
/// identities) render their display id but do not link — they are not a single memory to open.
function LinkedPairs({
  graph,
  base,
  cursor,
  navigate,
}: {
  graph: MemoryGraph;
  base: string;
  cursor: number;
  navigate: (path: string) => void;
}) {
  if (graph.links.length === 0) {
    return <p className="font-mono text-2xs text-ink-faint">no links for these relations</p>;
  }
  return (
    <section>
      <span className="font-mono text-2xs uppercase tracking-widest text-ink-faint">
        {`linked · ${graph.links.length}`}
      </span>
      <ul className="mt-2 flex flex-col gap-1 font-mono text-xs text-ink-soft">
        {graph.links.map((link, index) => (
          <li
            key={`${link.source}-${link.relation}-${link.target}-${index}`}
            className="flex items-baseline gap-2"
          >
            <MemoryLink name={link.source} base={base} cursor={cursor} navigate={navigate} />
            <span style={{ color: relationColor(link.relation) }}>{link.relation}</span>
            <MemoryLink name={link.target} base={base} cursor={cursor} navigate={navigate} />
          </li>
        ))}
      </ul>
    </section>
  );
}

/// A clickable memory name that navigates to the State view at the cursor. Virtual nodes (carrying
/// `members`) are shown as plain text — they are a class, not a single memory to open.
function MemoryLink({
  name,
  base,
  cursor,
  navigate,
}: {
  name: string;
  base: string;
  cursor: number;
  navigate: (path: string) => void;
}) {
  // A collapsed virtual node id ends in " (N)" — it is not a memory name to open.
  const isVirtualNode = /\(\d+\)$/.test(name);
  if (isVirtualNode) {
    return <span className="text-sage">{name}</span>;
  }
  return (
    <button
      onClick={() => navigate(statePath(base, cursor, name))}
      title={`Open ${name} in State`}
      className="text-clay underline-offset-2 transition-colors hover:text-ink hover:underline"
    >
      {name}
    </button>
  );
}

/// Splice expanded virtual nodes' members into the graph as satellite nodes. The class node stays
/// (so inter-class edges keep their anchor); its members appear as nodes linked to it by `same`
/// edges drawn as undirected dashed lines. No positions are seeded — the force layout's link force
/// pulls the members toward their class node, settling them as a cluster.
function expandVirtualNodes(graph: MemoryGraph, expanded: Set<string>): MemoryGraph {
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

function isVirtual(node: NodeObject): node is NodeObject & { members: string[] } {
  return (
    Array.isArray((node as MemoryGraphNode).members) &&
    (node as MemoryGraphNode).members!.length > 0
  );
}

function nodeLabel(node: NodeObject): string {
  if (isVirtual(node)) {
    return `${node.id} — ${node.members.join(", ")}`;
  }
  return String(node.id);
}

/// The bounding box for a pill-shaped node, in world-space units so it scales with the camera.
function nodeShape(
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

function cardinalityLabel(relation: RelationView): string {
  return `${relation.from_card}→${relation.to_card}`;
}

function namespaceOf(name: string): string {
  const slash = name.indexOf("/");
  return slash === -1 ? name : name.slice(0, slash);
}

/// A stable color for a relation name, derived from a hash so the legend swatch and the graph edge
/// match without a hand-maintained palette. `same` edges fall back to the sage accent, since they are
/// identity plumbing rather than a typed relation with a registry entry.
function relationColor(name: string, fallback?: string): string {
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
function readPalette() {
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
