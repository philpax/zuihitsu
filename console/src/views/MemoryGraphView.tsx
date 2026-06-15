import { useEffect, useRef, useState } from "react";
import { useNavigate } from "react-router-dom";
import ForceGraph2D from "react-force-graph-2d";

import type { Replica } from "../lib/replica.ts";
import type { MemoryGraphLink, MemoryGraphNode } from "../lib/memoryGraph.ts";
import { buildMemoryGraph } from "../lib/memoryGraph.ts";
import { useStreamBase } from "../lib/useStreamLocation.ts";
import { statePath } from "../lib/routes.ts";

/// The memory graph: every memory as a node, its typed relations as directed edges, its `same_as`
/// class as undirected dashed ones — a force-directed view of how the graph hangs together at the
/// timeline cursor. Folded with the rest of the views (it is keyed by the cursor in the workspace, so
/// it rebuilds at each fold), and a node opens that memory in the State view at the same point.
export function MemoryGraphView({ replica, cursor }: { replica: Replica; cursor: number }) {
  const navigate = useNavigate();
  const base = useStreamBase();
  const graph = buildMemoryGraph(replica);
  const palette = readPalette();

  // The force-graph canvas needs explicit pixel dimensions, so measure the container it fills.
  const wrap = useRef<HTMLDivElement>(null);
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

  if (graph.nodes.length === 0) {
    return (
      <div className="py-16 text-center text-sm text-ink-faint">
        No memories to graph at this point in the log.
      </div>
    );
  }

  return (
    <section className="flex flex-col gap-3">
      <div className="flex flex-wrap items-baseline justify-between gap-3">
        <h2 className="font-serif text-xl text-ink sm:text-2xl">Graph</h2>
        <div className="flex items-baseline gap-4 font-mono text-2xs text-ink-faint">
          <span>
            {graph.nodes.length} memories · {graph.links.length} links
          </span>
          <span className="flex items-baseline gap-1.5">
            <span className="text-clay">→</span> relation
          </span>
          <span className="flex items-baseline gap-1.5">
            <span className="text-sage">···</span> same as
          </span>
        </div>
      </div>
      <div ref={wrap} className="h-[68vh] w-full overflow-hidden border border-line bg-oat/20">
        {size.width > 0 && (
          <ForceGraph2D
            graphData={graph}
            width={size.width}
            height={size.height}
            backgroundColor="rgba(0,0,0,0)"
            nodeRelSize={4}
            nodeColor={(node: MemoryGraphNode) => (node.id === "self" ? palette.clay : palette.ink)}
            nodeLabel={(node: MemoryGraphNode) => node.id}
            linkColor={(link: MemoryGraphLink) => (link.same ? palette.sage : palette.clay)}
            linkLineDash={(link: MemoryGraphLink) => (link.same ? [3, 3] : null)}
            linkDirectionalArrowLength={(link: MemoryGraphLink) => (link.same ? 0 : 3)}
            linkDirectionalArrowRelPos={1}
            linkWidth={1}
            linkLabel={(link: MemoryGraphLink) => link.relation}
            nodeCanvasObjectMode={() => "after"}
            nodeCanvasObject={(node: NodeWithCoords, ctx, scale) => {
              const fontSize = 11 / scale;
              ctx.font = `${fontSize}px ui-monospace, monospace`;
              ctx.fillStyle = palette.inkSoft;
              ctx.textAlign = "center";
              ctx.textBaseline = "top";
              ctx.fillText(String(node.id), node.x ?? 0, (node.y ?? 0) + 5);
            }}
            onNodeClick={(node: MemoryGraphNode) =>
              navigate(statePath(base, cursor, String(node.id)))
            }
          />
        )}
      </div>
    </section>
  );
}

type NodeWithCoords = MemoryGraphNode & { x?: number; y?: number };

/// Read the Japandi tokens off the document once, so the canvas (which can only take concrete colors)
/// stays in step with the design tokens rather than hard-coding hexes.
let cachedPalette: { ink: string; inkSoft: string; clay: string; sage: string } | null = null;
function readPalette() {
  if (!cachedPalette) {
    const root = getComputedStyle(document.documentElement);
    const value = (name: string) => root.getPropertyValue(name).trim();
    cachedPalette = {
      ink: value("--color-ink"),
      inkSoft: value("--color-ink-soft"),
      clay: value("--color-clay"),
      sage: value("--color-sage"),
    };
  }
  return cachedPalette;
}
