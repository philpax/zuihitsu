import { createContext } from "react";

import type { TurnModel } from "./conversation.ts";
import { scanTurnRefs } from "./replica.ts";

// The non-component half of turn references (spec §Conversations → Transcript references): the
// lookup context the Conversation view fills, and the remark plugin that lifts scanned references
// out of an agent turn's Markdown. The chips themselves live in `components/TurnRefs.tsx`.

/// The URL scheme the remark plugin smuggles a reference through react-markdown with: a scanned ref
/// becomes a link node `turnref:<ulid>`, which the anchor override renders as a chip.
export const TURNREF_SCHEME = "turnref:";

/// Where a referenced turn lives in the folded log: the moment itself, the room that holds it (as
/// the sidebar's channel key, for the deep link), and its immediate neighbors for the hover preview.
export interface TurnRefTarget {
  turn: TurnModel;
  roomKey: string;
  /// The focal turn and up to two neighbors either side, in transcript order.
  window: TurnModel[];
  /// The focal turn's index within `window`.
  focusIndex: number;
}

/// Every folded turn by id, built by the Conversation view at the current cursor — so a chip
/// resolves against exactly what the timeline shows, and an id past the cursor reads as unknown.
export const TurnRefs = createContext<Map<string, TurnRefTarget>>(new Map());

/// The chip and popup label for a turn's speaker, matching the transcript's own labels.
export function speakerLabel(turn: TurnModel): string {
  if (turn.role === "Agent") return "the agent";
  if (turn.role === "System") return "system";
  return turn.speaker ?? "someone";
}

/// A minimal mdast node — just the fields the reference splitter walks, so the plugin does not pull
/// the mdast type packages in for four properties.
interface MdNode {
  type: string;
  value?: string;
  url?: string;
  children?: MdNode[];
}

/// A remark plugin that turns scanned references in an agent turn's Markdown into `turnref:` link
/// nodes, which the anchor override renders as chips. Operates on the mdast text nodes, so code
/// blocks and inline code are naturally untouched (their content is not a text node).
export function remarkTurnRefs() {
  return (tree: MdNode) => splitRefs(tree);
}

function splitRefs(node: MdNode): void {
  if (!node.children) return;
  const next: MdNode[] = [];
  for (const child of node.children) {
    // A GFM-autolinked deep-link URL becomes a chip for the whole link. A link the author labeled
    // (`[see here](…?turn=…)`) keeps its label and renders as an ordinary anchor, and no chip is
    // ever nested inside another anchor — so links are never descended into.
    if (child.type === "link") {
      const autolink =
        child.children?.length === 1 &&
        child.children[0].type === "text" &&
        child.children[0].value === child.url;
      const ids = autolink ? extractIds(child.url ?? "") : [];
      next.push(ids.length === 1 ? refLink(ids[0]) : child);
      continue;
    }
    if (child.type === "text" && child.value) {
      const segments = scanTurnRefs(child.value);
      if (segments.some((segment) => segment.kind === "ref")) {
        for (const segment of segments) {
          next.push(
            segment.kind === "prose" ? { type: "text", value: segment.text } : refLink(segment.id),
          );
        }
        continue;
      }
    }
    splitRefs(child);
    next.push(child);
  }
  node.children = next;
}

/// A `turnref:` link node for the anchor override to catch.
function refLink(id: string): MdNode {
  return {
    type: "link",
    url: TURNREF_SCHEME + id,
    children: [{ type: "text", value: id }],
  };
}

/// The turn ids a bare URL references, via the same scanner (a URL is one scannable token).
function extractIds(url: string): string[] {
  return scanTurnRefs(url).flatMap((segment) => (segment.kind === "ref" ? [segment.id] : []));
}
