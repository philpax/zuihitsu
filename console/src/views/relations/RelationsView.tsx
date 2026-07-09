import { useEffect, useRef, useState } from "react";
import { useNavigate, useSearchParams } from "react-router-dom";
import ForceGraph2D, {
  type ForceGraphMethods,
  type LinkObject,
  type NodeObject,
} from "react-force-graph-2d";

import type { Replica } from "../../lib/replica/replica.ts";
import type { MemoryId } from "../../types/MemoryId.ts";
import type { MemoryGraph } from "../../lib/model/memoryGraph.ts";
import {
  buildMemoryGraph,
  collapseSameAs,
  filterByRelations,
} from "../../lib/model/memoryGraph.ts";
import { useStreamBase } from "../../lib/nav/useStreamLocation.ts";
import { statePath } from "../../lib/nav/routes.ts";
import { MergeProposals } from "./MergeProposals.tsx";
import { LinkedPairs, RelationLegend } from "./Legend.tsx";
import { conversationNameById } from "../../components/EventDetail.tsx";
import {
  SIZES,
  expandVirtualNodes,
  isVirtual,
  nodeLabel,
  nodeShape,
  readPalette,
  relationColor,
} from "./graphUtilities.ts";

/// The operator's merge-decision hook, supplied only by the live agent frame when the cursor is at the
/// head — resolving a proposal authors an operator event, which the read-only eval viewer cannot do.
export interface MergeControls {
  resolve: (from: MemoryId, to: MemoryId, accept: boolean) => Promise<void>;
}

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
  const nameById = new Map(replica.memories("").map((m) => [m.id, m.name]));
  const convNameById = conversationNameById(replica.conversations());

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

  // A defensive copy of the filtered graph for `LinkedPairs`, so the force-graph library's
  // in-place mutation of node/link objects (it replaces `source`/`target` strings with node
  // object references) does not corrupt the data the list reads. The force graph receives
  // `graphData` (which may share references with `filtered` when no virtual nodes are expanded);
  // this copy stays pristine.
  const linkedPairsGraph: MemoryGraph = {
    nodes: filtered.nodes,
    links: filtered.links.map((link) => ({ ...link })),
  };

  return (
    <div className="flex flex-col gap-4">
      {/* The cross-platform merge proposals derived from the folded log — the operator's identity
          adjudication surface, above the relation graph the merges reshape. */}
      <MergeProposals
        proposals={proposals}
        base={base}
        cursor={cursor}
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
          <LinkedPairs
            graph={linkedPairsGraph}
            base={base}
            cursor={cursor}
            nameById={nameById}
            conversationNameById={convNameById}
          />
        </>
      )}
    </div>
  );
}
