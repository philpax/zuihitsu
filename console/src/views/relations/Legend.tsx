import type { RelationView } from "../../lib/model/graph.ts";
import type { MemoryGraph } from "../../lib/model/memoryGraph.ts";
import { statePath } from "../../lib/nav/routes.ts";
import { Checkbox } from "../../components/primitives.tsx";
import { cardinalityLabel, relationColor } from "./graphUtilities.ts";

/// The relation registry as a table. Each row is a toggle filter; clicking "all" clears the filter.
/// The swatch column matches the graph's edge color for the relation.
export function RelationLegend({
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
export function LinkedPairs({
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
