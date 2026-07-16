import { useState } from "react";

import type { RelationView } from "../../lib/model/graph.ts";
import type { MemoryGraph, MemoryGraphLink } from "../../lib/model/memoryGraph.ts";
import { isPrivate, tellerLabel, visibilityLabel } from "../../lib/model/labels.ts";
import { Checkbox } from "../../components/primitives.tsx";
import { ConversationRefLink, MemoryNameLink } from "../../components/eventDetailParts.tsx";
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
          <table className="w-full min-w-136 table-fixed border-collapse">
            <thead>
              <tr className="border-b border-line text-left font-mono text-2xs tracking-widest text-ink-faint uppercase">
                <th className="w-[20%] pr-2 pb-1 font-normal">name</th>
                <th className="w-[20%] pr-2 pb-1 font-normal">inverse</th>
                <th className="w-24 pr-2 pb-1 font-normal">card</th>
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
                    <td className="py-1 pr-2 pl-2.5 font-mono text-xs" style={{ color }}>
                      {relation.name}
                    </td>
                    <td className="py-1 pr-2 font-mono text-2xs text-ink-faint">
                      {relation.inverse}
                    </td>
                    <td className="py-1 pr-2 font-mono text-2xs text-ink-faint">
                      {cardinalityLabel(relation)}
                    </td>
                    <td className="py-1 text-2xs/snug text-ink-faint">
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
/// Non-public links carry a faint visibility tag after the triple. Clicking a row expands it to
/// show the link's full provenance: who asserted it, where it was told, and its visibility posture.
export function LinkedPairs({
  graph,
  base,
  cursor,
  nameById,
  conversationNameById,
}: {
  graph: MemoryGraph;
  base: string;
  cursor: number;
  nameById: Map<string, string>;
  conversationNameById: Map<string, string>;
}) {
  if (graph.links.length === 0) {
    return <p className="font-mono text-2xs text-ink-faint">no links for these relations</p>;
  }
  return (
    <section>
      <span className="font-mono text-2xs tracking-widest text-ink-faint uppercase">
        {`linked · ${graph.links.length}`}
      </span>
      <ul className="mt-2 flex flex-col gap-0.5 font-mono text-xs text-ink-soft">
        {graph.links.map((link, index) => (
          <LinkRow
            key={`${link.source}-${link.relation}-${link.target}-${index}`}
            link={link}
            base={base}
            cursor={cursor}
            nameById={nameById}
            conversationNameById={conversationNameById}
          />
        ))}
      </ul>
    </section>
  );
}

/// One link row in the `LinkedPairs` list. The summary line is always visible; clicking it
/// expands a detail panel with the link's provenance (told by, told in, visibility).
function LinkRow({
  link,
  base,
  cursor,
  nameById,
  conversationNameById,
}: {
  link: MemoryGraphLink;
  base: string;
  cursor: number;
  nameById: Map<string, string>;
  conversationNameById: Map<string, string>;
}) {
  const [expanded, setExpanded] = useState(false);
  const hasDetail = link.told_by !== null || link.told_in !== null || isPrivate(link.visibility);
  return (
    <li>
      <div
        className={
          "flex items-baseline gap-2 " + (hasDetail ? "cursor-pointer hover:bg-oat/30" : "")
        }
        onClick={() => hasDetail && setExpanded(!expanded)}
      >
        <MemoryNameLink name={link.source} base={base} seq={cursor} />
        <span style={{ color: relationColor(link.relation) }}>{link.relation}</span>
        <MemoryNameLink name={link.target} base={base} seq={cursor} />
        {isPrivate(link.visibility) && (
          <span className="text-2xs text-clay/70">
            {visibilityLabel(link.visibility, nameById)}
          </span>
        )}
        {hasDetail && <span className="text-2xs text-ink-faint">{expanded ? "▾" : "▸"}</span>}
      </div>
      {expanded && hasDetail && (
        <dl className="ml-4 flex flex-col gap-0.5 py-1 text-2xs text-ink-faint">
          <div className="flex gap-2">
            <dt>visibility</dt>
            <dd className={isPrivate(link.visibility) ? "text-clay" : undefined}>
              {visibilityLabel(link.visibility, nameById)}
            </dd>
          </div>
          {link.told_by && (
            <div className="flex gap-2">
              <dt>told by</dt>
              <dd>
                {(() => {
                  const label = tellerLabel(link.told_by!, nameById);
                  if (link.told_by === "Agent" || link.told_by === "Bootstrap") {
                    return label;
                  }
                  const participantId = link.told_by.Participant;
                  const name = nameById.get(participantId) ?? participantId;
                  return <MemoryNameLink name={name} base={base} seq={cursor} />;
                })()}
              </dd>
            </div>
          )}
          {link.told_in && (
            <div className="flex gap-2">
              <dt>told in</dt>
              <dd>
                <ConversationRefLink
                  value={link.told_in}
                  nameById={nameById}
                  conversationNameById={conversationNameById}
                  base={base}
                />
              </dd>
            </div>
          )}
        </dl>
      )}
    </li>
  );
}
