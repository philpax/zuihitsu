import { useEffect, useRef, useState } from "react";
import ForceGraph2D, {
  type ForceGraphMethods,
  type LinkObject,
  type NodeObject,
} from "react-force-graph-2d";

import type { Replica } from "../../lib/replica/replica.ts";
import type { MemoryId } from "@zuihitsu/wire/types/MemoryId.ts";
import type { MemoryGraph } from "../../lib/model/memoryGraph.ts";
import {
  buildMemoryGraph,
  collapseSameAs,
  filterByRelations,
} from "../../lib/model/memoryGraph.ts";
import { useNavigate } from "../../lib/nav/historyContext.ts";
import { useStream } from "../../lib/nav/useStreamLocation.ts";
import { MergeProposals } from "./MergeProposals.tsx";
import { LinkedPairs, RelationLegend } from "./Legend.tsx";
import { conversationNameById } from "../../lib/model/conversationNameById.ts";
import { Segmented } from "../../components/primitives.tsx";
import { relationColor } from "../../lib/format/relationColor.ts";
import {
  SIZES,
  expandVirtualNodes,
  isVirtual,
  nodeLabel,
  nodeShape,
  readPalette,
} from "./graphUtilities.ts";

/// The operator's merge-decision hooks, supplied only by the live agent frame when the cursor is at the
/// head — each authors an operator event, which the read-only eval viewer cannot do. `resolve` confirms
/// a pending proposal; `unmerge` retracts a merge that was already made, splitting the class back apart;
/// `designatePrimary` pins (or releases) which stub a merged class resolves through.
export interface MergeControls {
  resolve: (from: MemoryId, to: MemoryId) => Promise<void>;
  unmerge: (from: MemoryId, to: MemoryId) => Promise<void>;
  designatePrimary: (memory: MemoryId, designated: boolean) => Promise<void>;
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
///
/// Two subtabs split the surface: "All" holds the relation registry, the graph, and the linked-pairs
/// list; "Identity Merges" holds the cross-platform merge-proposal surface. The active subtab rides in
/// the URL as the view's selection segment (`/…/relations/<subtab>`), so it is a shareable deep link and
/// browser back and forward walk it — the same register the Settings view's section uses. Switching
/// subtab carries the relation filters (which live in the search) forward, so returning to "All" finds
/// them intact; an unknown segment falls back to "all". The `SubtabId` union guards the segment, so a
/// stale or malformed value degrades to the default rather than blanking the view.
const SUBTABS = [
  { id: "all", label: "All" },
  { id: "merges", label: "Identity Merges" },
] as const;

type SubtabId = (typeof SUBTABS)[number]["id"];

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
  const { search, link, patchSearch, selection } = useStream();
  const palette = readPalette();
  // The active subtab is the URL selection segment, defaulting to "all" when absent or unrecognized.
  const subtab: SubtabId = SUBTABS.some((entry) => entry.id === selection)
    ? (selection as SubtabId)
    : "all";

  // The linked-pairs list windows a long newest-first list, so it carries a page cursor. Changing the
  // subtab or the relation/collapse filters reshuffles which links are listed, so those handlers reset
  // the cursor to the first page rather than stranding the reader on a now-out-of-range page.
  const [page, setPage] = useState(0);

  // URL state: the selected relations (empty = all), the `same_as` collapse toggle (default on),
  // and the comma-joined set of expanded virtual-node ids. Defaults are applied when the param is
  // absent so the first visit to the tab is the intended de-cluttered overview.
  const selected = search.relations
    ? new Set(search.relations.split(",").filter(Boolean))
    : new Set<string>();
  const sameAs = search.sameAs !== "off";
  const expanded = search.expand ? new Set(search.expand.split(",")) : new Set<string>();

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

  // The filter and display toggles are continuous interaction, so each replaces (rather than pushes) a
  // history entry via `patchSearch`, mutating only its own search key.
  function toggleRelation(name: string) {
    patchSearch((prev) => ({ ...prev, relations: toggleCsv(prev.relations, name) }));
    setPage(0);
  }

  function clearRelations() {
    patchSearch((prev) => ({ ...prev, relations: undefined }));
    setPage(0);
  }

  function toggleSameAs(on: boolean) {
    patchSearch((prev) => ({ ...prev, sameAs: on ? undefined : "off" }));
    setPage(0);
  }

  function toggleExpand(id: string) {
    patchSearch((prev) => ({ ...prev, expand: toggleCsv(prev.expand, id) }));
  }

  // The force-graph canvas needs explicit pixel dimensions, so measure the container it fills. The
  // wrap div is only in the DOM on the "all" subtab with nodes to draw, so the observer effect keys
  // off that condition: it re-attaches whenever the container appears and tears down when it leaves.
  // Keying mount-only ([]) would strand the graph whenever the container is absent on the mount render
  // — a non-"all" subtab, or a live agent whose log has not yet yielded a memory — because the effect
  // would early-return on the null ref and never re-run, leaving `size` at zero so the width-gated
  // `ForceGraph2D` never renders.
  const showGraph = subtab === "all" && raw.nodes.length > 0;
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
  }, [showGraph]);

  const graphData = expandVirtualNodes(filtered, expanded);
  const proposals = replica.mergeProposals();

  // A defensive copy of the filtered graph for `LinkedPairs`, so the force-graph library's
  // in-place mutation of node/link objects (it replaces `source`/`target` strings with node
  // object references) does not corrupt the data the list reads. The force graph receives
  // `graphData` (which may share references with `filtered` when no virtual nodes are expanded);
  // this copy stays pristine. The links are ordered newest-first by creation instant, so the
  // paginated list leads with the most recent edges; a synthetic `same as` edge has no instant and
  // sorts last.
  const linkedPairsGraph: MemoryGraph = {
    nodes: filtered.nodes,
    links: filtered.links
      .map((link) => ({ ...link }))
      .sort((a, b) => (b.asserted_at ?? -1) - (a.asserted_at ?? -1)),
  };

  return (
    <div className="flex flex-col gap-4">
      <Segmented
        options={SUBTABS}
        value={subtab}
        onChange={(id) => {
          // A subtab move is navigation (pushed), carrying the current search — the relation filters,
          // the collapse toggle, the expanded set, and the cursor — so returning to "All" restores them.
          navigate(link.view("relations", { selection: id, search }));
          setPage(0);
        }}
      />

      {subtab === "merges" ? (
        // The cross-platform merge proposals derived from the folded log — the operator's identity
        // confirmation surface. `MergeProposals` renders nothing when there are none, so the tab
        // supplies its own empty state.
        proposals.length === 0 ? (
          <div className="py-16 text-center text-sm text-ink-faint">
            No identity merges proposed at this point in the log.
          </div>
        ) : (
          <MergeProposals
            proposals={proposals}
            cursor={cursor}
            onResolve={merge?.resolve}
            onUnmerge={merge?.unmerge}
            onDesignatePrimary={merge?.designatePrimary}
          />
        )
      ) : raw.nodes.length === 0 ? (
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
                    navigate(link.state(String(node.id), { seq: cursor }));
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
            cursor={cursor}
            nameById={nameById}
            conversationNameById={convNameById}
            page={page}
            onPage={setPage}
          />
        </>
      )}
    </div>
  );
}

/// Toggle one item in a comma-joined set carried in a search key, returning the new value — or
/// `undefined` when the set empties, so the key drops out of the URL rather than lingering blank.
function toggleCsv(csv: string | undefined, item: string): string | undefined {
  const set = csv ? new Set(csv.split(",").filter(Boolean)) : new Set<string>();
  if (set.has(item)) set.delete(item);
  else set.add(item);
  return set.size === 0 ? undefined : [...set].join(",");
}
